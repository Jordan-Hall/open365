//! The wgpu side: device setup, the renderer that draws a [`DrawList`], and offscreen targets
//! with readback for headless tests.

use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};

use crate::assets::Assets;
use crate::atlas::Atlas;
use crate::color::{Color, srgb_to_linear};
use crate::draw::{DrawList, Vertex};
use crate::error::GfxError;

/// Format used for offscreen targets: 8-bit RGBA with sRGB encoding, so blending happens in
/// linear light and readback bytes are exactly what a screen would show.
pub const OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Picks the swap-chain format: an sRGB one when offered (gamma-correct blending), else the
/// first offered format.
pub fn choose_surface_format(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    formats
        .iter()
        .copied()
        .find(wgpu::TextureFormat::is_srgb)
        .or_else(|| formats.first().copied())
}

/// Instance, adapter, device and queue.
#[derive(Debug)]
pub struct GpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl GpuContext {
    /// A context with no window. Falls back to a software adapter when no GPU is present;
    /// returns [`GfxError::NoAdapter`] when there is nothing at all (tests skip on this).
    pub fn headless() -> Result<Self, GfxError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        Self::from_instance(instance, None)
    }

    /// A context on `instance` whose adapter can present to `surface` (when given).
    pub fn from_instance(
        instance: wgpu::Instance,
        surface: Option<&wgpu::Surface<'_>>,
    ) -> Result<Self, GfxError> {
        let request = |force_fallback_adapter: bool| {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter,
                compatible_surface: surface,
                ..Default::default()
            }))
        };
        let adapter = request(false)
            .or_else(|_| request(true))
            .map_err(|e| GfxError::NoAdapter(e.to_string()))?;
        let (device, queue) = pollster::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("open365-gfx"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults()
                    .using_resolution(adapter.limits())
                    .using_alignment(adapter.limits()),
                experimental_features: wgpu::ExperimentalFeatures::default(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            }),
        )
        .map_err(|e| GfxError::Device(e.to_string()))?;
        Ok(Self {
            instance,
            adapter,
            device,
            queue,
        })
    }
}

/// Text coverage shaping, applied in the fragment shader. The defaults are the identity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderParams {
    /// Coverage exponent for dark ink (`coverage^(1/gamma)`); 1.0 leaves coverage alone.
    pub text_gamma: f32,
    /// Extra mid-coverage boost for dark ink; 0.0 leaves coverage alone.
    pub text_contrast: f32,
    /// Like `text_gamma` for light ink (white text on a dark background).
    pub text_gamma_light: f32,
    /// Like `text_contrast` for light ink.
    pub text_contrast_light: f32,
}

impl RenderParams {
    /// Ink-mass fit to Office text at Segoe UI 9 pt / 14 dip, 125 % scale, `Hinting::Normal`
    /// (`examples/fit_text.rs`, 2026-10-11, one machine). Light ink (white tab labels on a dark
    /// background, grayscale) matches Office's total ink at gamma 0.65. Dark ink (cell text on
    /// white, ClearType, LCD path) matches at gamma 2.50. Per-pixel residual against the capture is
    /// still about 29 levels RMS because glyph advances and hinting differ; not yet within the
    /// 1 px bar. Dark-ink grayscale was not captured and shares the LCD value.
    pub fn office_fitted() -> Self {
        Self {
            text_gamma: 2.5,
            text_contrast: 0.0,
            text_gamma_light: 0.65,
            text_contrast_light: 0.0,
        }
    }
}

impl Default for RenderParams {
    fn default() -> Self {
        Self {
            text_gamma: 1.0,
            text_contrast: 0.0,
            text_gamma_light: 1.0,
            text_contrast_light: 0.0,
        }
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Globals {
    viewport: [f32; 2],
    atlas_size: f32,
    srgb_encode: f32,
    text_gamma: f32,
    text_contrast: f32,
    text_gamma_light: f32,
    text_contrast_light: f32,
}

#[derive(Debug)]
struct AtlasTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

fn make_atlas_texture(
    device: &wgpu::Device,
    size: u32,
    format: wgpu::TextureFormat,
    label: &str,
) -> AtlasTexture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    AtlasTexture { texture, view }
}

fn upload_dirty(queue: &wgpu::Queue, tex: &AtlasTexture, atlas: &mut Atlas) {
    let d = atlas.take_dirty();
    if d.is_empty() {
        return;
    }
    let ch = atlas.channels();
    let stride = atlas.width() * ch;
    let start = (d.y0 * stride + d.x0 * ch) as usize;
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex.texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: d.x0,
                y: d.y0,
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        },
        &atlas.pixels()[start..],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(stride),
            rows_per_image: Some(d.height()),
        },
        wgpu::Extent3d {
            width: d.width(),
            height: d.height(),
            depth_or_array_layers: 1,
        },
    );
}

/// Draws [`DrawList`]s into texture views of one colour format.
#[derive(Debug)]
pub struct Renderer {
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    lcd_pipelines: [wgpu::RenderPipeline; 3],
    globals: wgpu::Buffer,
    globals_bind: wgpu::BindGroup,
    atlas_bind: wgpu::BindGroup,
    clip_layout: wgpu::BindGroupLayout,
    clip_buffer: wgpu::Buffer,
    clip_bind: wgpu::BindGroup,
    clip_slots: u64,
    mask: AtlasTexture,
    lcd: AtlasTexture,
    color: AtlasTexture,
    atlas_size: u32,
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: u64,
    index_buffer: wgpu::Buffer,
    index_capacity: u64,
    params: RenderParams,
}

fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    static ATTRS: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
        0 => Float32x2,
        1 => Float32x2,
        2 => Unorm8x4,
        3 => Uint32,
        4 => Float32x4,
    ];
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &ATTRS,
    }
}

const INITIAL_VERTICES: u64 = 4096;
/// Size of one clip uniform; slots are `CLIP_STRIDE` apart (the largest dynamic-offset alignment).
const CLIP_UNIFORM_BYTES: u64 = 32;
const CLIP_STRIDE: u64 = 256;

fn make_clip_buffer(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    slots: u64,
) -> (wgpu::Buffer, wgpu::BindGroup) {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gfx-clip"),
        size: slots * CLIP_STRIDE,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("gfx-clip"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer: &buffer,
                offset: 0,
                size: NonZeroU64::new(CLIP_UNIFORM_BYTES),
            }),
        }],
    });
    (buffer, bind)
}

impl Renderer {
    /// Builds pipelines for `format`. `atlas_size` must equal [`Assets::atlas_size`] of the
    /// assets later passed to [`render`](Self::render).
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, atlas_size: u32) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("open365-gfx"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gfx-globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: NonZeroU64::new(std::mem::size_of::<Globals>() as u64),
                },
                count: None,
            }],
        });
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gfx-atlases"),
            entries: &[
                tex_entry(0),
                tex_entry(1),
                tex_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let clip_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gfx-clip"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: NonZeroU64::new(CLIP_UNIFORM_BYTES),
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gfx-layout"),
            bind_group_layouts: &[
                Some(&globals_layout),
                Some(&atlas_layout),
                Some(&clip_layout),
            ],
            immediate_size: 0,
        });

        let premultiplied = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let straight = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        let make = |label: &str,
                    entry: &str,
                    blend: wgpu::BlendState,
                    write_mask: wgpu::ColorWrites,
                    constants: &[(&str, f64)]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[Some(vertex_layout())],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants,
                        zero_initialize_workgroup_memory: false,
                    },
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(blend),
                        write_mask,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = make(
            "gfx-main",
            "fs_main",
            premultiplied,
            wgpu::ColorWrites::ALL,
            &[],
        );
        let lcd_pipelines = [
            make(
                "gfx-lcd-r",
                "fs_lcd",
                straight,
                wgpu::ColorWrites::RED,
                &[("LCD_CHANNEL", 0.0)],
            ),
            make(
                "gfx-lcd-g",
                "fs_lcd",
                straight,
                wgpu::ColorWrites::GREEN,
                &[("LCD_CHANNEL", 1.0)],
            ),
            make(
                "gfx-lcd-b",
                "fs_lcd",
                straight,
                wgpu::ColorWrites::BLUE,
                &[("LCD_CHANNEL", 2.0)],
            ),
        ];

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gfx-globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gfx-globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });

        let mask = make_atlas_texture(device, atlas_size, wgpu::TextureFormat::R8Unorm, "gfx-mask");
        let lcd = make_atlas_texture(
            device,
            atlas_size,
            wgpu::TextureFormat::Rgba8Unorm,
            "gfx-lcd",
        );
        let color = make_atlas_texture(
            device,
            atlas_size,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            "gfx-color",
        );
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gfx-linear"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let atlas_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gfx-atlases"),
            layout: &atlas_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&mask.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&lcd.view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&color.view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let clip_slots = 16;
        let (clip_buffer, clip_bind) = make_clip_buffer(device, &clip_layout, clip_slots);
        let vertex_capacity = INITIAL_VERTICES * std::mem::size_of::<Vertex>() as u64;
        let index_capacity = INITIAL_VERTICES * 6 * 4;
        Self {
            format,
            pipeline,
            lcd_pipelines,
            globals,
            globals_bind,
            atlas_bind,
            clip_layout,
            clip_buffer,
            clip_bind,
            clip_slots,
            mask,
            lcd,
            color,
            atlas_size,
            vertex_buffer: Self::buffer(
                device,
                "gfx-vertices",
                vertex_capacity,
                wgpu::BufferUsages::VERTEX,
            ),
            vertex_capacity,
            index_buffer: Self::buffer(
                device,
                "gfx-indices",
                index_capacity,
                wgpu::BufferUsages::INDEX,
            ),
            index_capacity,
            params: RenderParams::default(),
        }
    }

    fn buffer(
        device: &wgpu::Device,
        label: &str,
        size: u64,
        usage: wgpu::BufferUsages,
    ) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: usage | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    pub fn set_params(&mut self, params: RenderParams) {
        self.params = params;
    }

    /// Uploads changed atlas regions and geometry, then draws `list` into `target`. With
    /// `clear` the target is cleared first, otherwise its contents are kept. `target` must be
    /// `list.size()` pixels and of [`format`](Self::format).
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &mut Assets,
        list: &DrawList,
        target: &wgpu::TextureView,
        clear: Option<Color>,
    ) -> Result<(), GfxError> {
        if assets.atlas_size() != self.atlas_size {
            return Err(GfxError::Surface(format!(
                "assets atlas is {} px but the renderer was built for {}",
                assets.atlas_size(),
                self.atlas_size
            )));
        }
        upload_dirty(queue, &self.mask, assets.mask_atlas());
        upload_dirty(queue, &self.lcd, assets.lcd_atlas());
        upload_dirty(queue, &self.color, assets.color_atlas());

        let (width, height) = list.size();
        let globals = Globals {
            viewport: [width as f32, height as f32],
            atlas_size: self.atlas_size as f32,
            srgb_encode: if self.format.is_srgb() { 0.0 } else { 1.0 },
            text_gamma: self.params.text_gamma,
            text_contrast: self.params.text_contrast,
            text_gamma_light: self.params.text_gamma_light,
            text_contrast_light: self.params.text_contrast_light,
        };
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&globals));

        let vbytes: &[u8] = bytemuck::cast_slice(list.vertices());
        let ibytes: &[u8] = bytemuck::cast_slice(list.indices());
        if vbytes.len() as u64 > self.vertex_capacity {
            self.vertex_capacity = (vbytes.len() as u64).next_power_of_two();
            self.vertex_buffer = Self::buffer(
                device,
                "gfx-vertices",
                self.vertex_capacity,
                wgpu::BufferUsages::VERTEX,
            );
        }
        if ibytes.len() as u64 > self.index_capacity {
            self.index_capacity = (ibytes.len() as u64).next_power_of_two();
            self.index_buffer = Self::buffer(
                device,
                "gfx-indices",
                self.index_capacity,
                wgpu::BufferUsages::INDEX,
            );
        }
        if !vbytes.is_empty() {
            queue.write_buffer(&self.vertex_buffer, 0, vbytes);
            queue.write_buffer(&self.index_buffer, 0, ibytes);
        }

        // Slot 0 is "no mask"; every batch with a mask gets its own slot.
        let masked = list.batches().iter().filter(|b| b.mask.is_some()).count() as u64 + 1;
        if masked > self.clip_slots {
            self.clip_slots = masked.next_power_of_two();
            let (b, g) = make_clip_buffer(device, &self.clip_layout, self.clip_slots);
            self.clip_buffer = b;
            self.clip_bind = g;
        }
        let mut clip_bytes = vec![0u8; (masked * CLIP_STRIDE) as usize];
        let mut slot_of = Vec::with_capacity(list.batches().len());
        let mut next_slot = 1u64;
        for b in list.batches() {
            match b.mask {
                Some(m) => {
                    let words: [f32; 8] = [
                        m.x as f32,
                        m.y as f32,
                        m.ox as f32,
                        m.oy as f32,
                        m.w as f32,
                        m.h as f32,
                        1.0,
                        0.0,
                    ];
                    let at = (next_slot * CLIP_STRIDE) as usize;
                    clip_bytes[at..at + 32].copy_from_slice(bytemuck::cast_slice(&words));
                    slot_of.push(next_slot);
                    next_slot += 1;
                }
                None => slot_of.push(0),
            }
        }
        queue.write_buffer(&self.clip_buffer, 0, &clip_bytes);

        let load = match clear {
            Some(c) => wgpu::LoadOp::Clear(wgpu::Color {
                r: f64::from(srgb_to_linear(c.r)),
                g: f64::from(srgb_to_linear(c.g)),
                b: f64::from(srgb_to_linear(c.b)),
                a: f64::from(c.a) / 255.0,
            }),
            None => wgpu::LoadOp::Load,
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gfx-frame"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gfx-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !list.is_empty() {
                pass.set_bind_group(0, &self.globals_bind, &[]);
                pass.set_bind_group(1, &self.atlas_bind, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..vbytes.len() as u64));
                pass.set_index_buffer(
                    self.index_buffer.slice(..ibytes.len() as u64),
                    wgpu::IndexFormat::Uint32,
                );
                for (batch, slot) in list.batches().iter().zip(&slot_of) {
                    pass.set_bind_group(2, &self.clip_bind, &[(*slot * CLIP_STRIDE) as u32]);
                    let clip = batch
                        .clip
                        .intersect(&crate::geom::PixelRect::new(0, 0, width, height));
                    if clip.is_empty() {
                        continue;
                    }
                    pass.set_scissor_rect(clip.x0, clip.y0, clip.width(), clip.height());
                    let range = batch.first_index..batch.first_index + batch.index_count;
                    if batch.lcd {
                        for p in &self.lcd_pipelines {
                            pass.set_pipeline(p);
                            pass.draw_indexed(range.clone(), 0, 0..1);
                        }
                    } else {
                        pass.set_pipeline(&self.pipeline);
                        pass.draw_indexed(range, 0, 0..1);
                    }
                }
            }
        }
        queue.submit([encoder.finish()]);
        Ok(())
    }
}

/// A colour texture to render into and read back, for headless tests.
#[derive(Debug)]
pub struct Offscreen {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
}

impl Offscreen {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gfx-offscreen"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: OFFSCREEN_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            width,
            height,
        }
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Copies the texture back as tightly packed RGBA8 (sRGB-encoded, as displayed).
    pub fn read_rgba(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<Vec<u8>, GfxError> {
        let unpadded = self.width * 4;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded = unpadded.div_ceil(align) * align;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gfx-readback"),
            size: u64::from(padded) * u64::from(self.height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gfx-readback"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| GfxError::Readback(e.to_string()))?;
        rx.recv()
            .map_err(|e| GfxError::Readback(e.to_string()))?
            .map_err(|e| GfxError::Readback(e.to_string()))?;
        let mapped = buffer
            .slice(..)
            .get_mapped_range()
            .map_err(|e| GfxError::Readback(e.to_string()))?;
        let mut out = Vec::with_capacity((unpadded * self.height) as usize);
        for row in mapped.chunks_exact(padded as usize) {
            out.extend_from_slice(&row[..unpadded as usize]);
        }
        Ok(out)
    }
}

/// Pixel `(x, y)` of tightly packed RGBA8 data `width` pixels wide.
pub fn pixel_at(rgba: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * width + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_format_prefers_srgb() {
        use wgpu::TextureFormat as F;
        assert_eq!(
            choose_surface_format(&[F::Bgra8Unorm, F::Bgra8UnormSrgb]),
            Some(F::Bgra8UnormSrgb)
        );
        assert_eq!(
            choose_surface_format(&[F::Rgba16Float, F::Bgra8Unorm]),
            Some(F::Rgba16Float)
        );
        assert_eq!(choose_surface_format(&[]), None);
    }

    #[test]
    fn fitted_params_keep_defaults_for_contrast() {
        let p = RenderParams::office_fitted();
        assert_eq!((p.text_contrast, p.text_contrast_light), (0.0, 0.0));
        assert!(p.text_gamma > 1.0 && p.text_gamma_light < 1.0);
    }

    #[test]
    fn pixel_at_indexes_rows() {
        let data: Vec<u8> = (0..16).collect();
        assert_eq!(pixel_at(&data, 2, 1, 1), [12, 13, 14, 15]);
    }
}
