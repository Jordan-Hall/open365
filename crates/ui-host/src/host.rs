//! The winit event loop: window, wgpu surface, DPI, frame scheduling, AccessKit and IME.

use std::sync::Arc;
use std::time::Instant;

use accesskit_winit::{Adapter, WindowEvent as AccessEvent};
use open365_gfx::{Assets, DrawList, GfxError, GpuContext, Renderer, choose_surface_format};
use thiserror::Error;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::app::{Cx, Frame, UiApp, UiEvent};
use crate::ime;
use crate::schedule::Scheduler;
use crate::theme::{ColorScheme, SystemMetrics, TitleBarTheme};

/// Window and renderer settings for [`run`].
#[derive(Debug, Clone)]
pub struct HostConfig {
    pub title: String,
    /// Initial client size in logical pixels.
    pub inner_size: (f64, f64),
    pub min_inner_size: Option<(f64, f64)>,
    pub title_bar: TitleBarTheme,
    /// Present with vsync (`Fifo`) when true, immediately otherwise.
    pub vsync: bool,
    /// Side of the glyph/icon/image atlas textures, in pixels.
    pub atlas_size: u32,
    /// Native title bar and borders. `false` leaves the application to draw its own caption
    /// (it can use `Window::drag_window`, `drag_resize_window` and the window controls).
    pub decorations: bool,
    /// Initial position of the window in logical pixels; `None` lets the system place it.
    pub position: Option<(f64, f64)>,
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            title: "Open365".into(),
            inner_size: (1280.0, 800.0),
            min_inner_size: Some((320.0, 240.0)),
            title_bar: TitleBarTheme::FollowSystem,
            vsync: true,
            atlas_size: 2048,
            decorations: true,
            position: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum HostError {
    #[error("event loop: {0}")]
    EventLoop(String),
    #[error("window: {0}")]
    Window(String),
    #[error(transparent)]
    Gfx(#[from] GfxError),
}

#[derive(Debug)]
enum UserEvent {
    AccessKit(accesskit_winit::Event),
}

impl From<accesskit_winit::Event> for UserEvent {
    fn from(e: accesskit_winit::Event) -> Self {
        Self::AccessKit(e)
    }
}

#[derive(Debug)]
struct Gfx {
    gpu: GpuContext,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: Renderer,
    assets: Assets,
    list: DrawList,
}

struct Live {
    window: Arc<Window>,
    adapter: Adapter,
    gfx: Gfx,
    ime_allowed: bool,
    scheme: ColorScheme,
    metrics: SystemMetrics,
}

struct Host<A: UiApp> {
    config: HostConfig,
    app: A,
    proxy: winit::event_loop::EventLoopProxy<UserEvent>,
    live: Option<Live>,
    scheduler: Scheduler,
    error: Option<HostError>,
}

/// Opens the window and runs `app` until it is closed.
pub fn run<A: UiApp>(config: HostConfig, app: A) -> Result<(), HostError> {
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|e| HostError::EventLoop(e.to_string()))?;
    // Sleep until something happens; redraws and timers are requested explicitly.
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut host = Host {
        config,
        app,
        proxy: event_loop.create_proxy(),
        live: None,
        scheduler: Scheduler::default(),
        error: None,
    };
    event_loop
        .run_app(&mut host)
        .map_err(|e| HostError::EventLoop(e.to_string()))?;
    host.error.map_or(Ok(()), Err)
}

fn make_cx<'a>(
    window: &'a Window,
    adapter: &'a mut Adapter,
    scheduler: &'a mut Scheduler,
    scheme: ColorScheme,
    metrics: SystemMetrics,
) -> Cx<'a> {
    Cx {
        window,
        adapter,
        scheduler,
        now: Instant::now(),
        scheme,
        scale_factor: window.scale_factor(),
        metrics,
    }
}

fn configure(gfx: &mut Gfx, width: u32, height: u32) {
    gfx.config.width = width.max(1);
    gfx.config.height = height.max(1);
    gfx.surface.configure(&gfx.gpu.device, &gfx.config);
}

impl<A: UiApp> Host<A> {
    fn create(&mut self, event_loop: &ActiveEventLoop) -> Result<Live, HostError> {
        let c = &self.config;
        let mut attrs = Window::default_attributes()
            .with_title(c.title.clone())
            .with_inner_size(LogicalSize::new(c.inner_size.0, c.inner_size.1))
            .with_theme(c.title_bar.to_winit())
            .with_decorations(c.decorations)
            // Hidden until the AccessKit adapter exists, as the adapter requires.
            .with_visible(false);
        if let Some((x, y)) = c.position {
            attrs = attrs.with_position(LogicalPosition::new(x, y));
        }
        if let Some((w, h)) = c.min_inner_size {
            attrs = attrs.with_min_inner_size(LogicalSize::new(w, h));
        }
        let window = event_loop
            .create_window(attrs)
            .map_err(|e| HostError::Window(e.to_string()))?;
        let adapter = Adapter::with_event_loop_proxy(event_loop, &window, self.proxy.clone());
        window.set_visible(true);
        let window = Arc::new(window);

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle(
            Box::new(event_loop.owned_display_handle()),
        ));
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| GfxError::Surface(e.to_string()))?;
        let gpu = GpuContext::from_instance(instance, Some(&surface))?;
        let caps = surface.get_capabilities(&gpu.adapter);
        let format = choose_surface_format(&caps.formats)
            .ok_or_else(|| GfxError::Surface("surface reports no formats".into()))?;
        let size = window.inner_size();
        let present_mode = if c.vsync {
            wgpu::PresentMode::Fifo
        } else if caps.present_modes.contains(&wgpu::PresentMode::Immediate) {
            wgpu::PresentMode::Immediate
        } else {
            wgpu::PresentMode::Fifo
        };
        let alpha_mode = caps
            .alpha_modes
            .iter()
            .copied()
            .find(|m| *m == wgpu::CompositeAlphaMode::Opaque)
            .unwrap_or(caps.alpha_modes[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::default(),
        };
        surface.configure(&gpu.device, &config);
        let assets = Assets::new(c.atlas_size);
        let renderer = Renderer::new(&gpu.device, format, assets.atlas_size());
        let scheme = window.theme().map_or(ColorScheme::Light, ColorScheme::from);
        let list = DrawList::new(config.width, config.height);
        Ok(Live {
            window,
            adapter,
            gfx: Gfx {
                gpu,
                surface,
                config,
                renderer,
                assets,
                list,
            },
            ime_allowed: false,
            scheme,
            metrics: SystemMetrics::read(),
        })
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: HostError) {
        self.error = Some(error);
        event_loop.exit();
    }

    fn redraw(&mut self) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        let Live {
            window,
            adapter,
            gfx,
            ime_allowed,
            scheme,
            metrics,
        } = live;
        let size = window.inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        if (gfx.config.width, gfx.config.height) != (size.width, size.height) {
            configure(gfx, size.width, size.height);
        }
        let (frame, stale) = match gfx.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => (t, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => (t, true),
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                configure(gfx, size.width, size.height);
                self.scheduler.request_redraw();
                return;
            }
            // Timeout, occluded or an unknown state: skip this frame.
            _ => return,
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.app.assets().unwrap_or(&mut gfx.assets).begin_frame();
        gfx.list.reset(size.width, size.height);
        {
            let mut cx = make_cx(window, adapter, &mut self.scheduler, *scheme, *metrics);
            let mut f = Frame {
                list: &mut gfx.list,
                assets: &mut gfx.assets,
                size_px: (size.width, size.height),
            };
            self.app.paint(&mut cx, &mut f);
        }
        let assets = self.app.assets().unwrap_or(&mut gfx.assets);
        let result = gfx.renderer.render(
            &gfx.gpu.device,
            &gfx.gpu.queue,
            assets,
            &gfx.list,
            &view,
            Some(open365_gfx::Color::WHITE),
        );
        window.pre_present_notify();
        gfx.gpu.queue.present(frame);
        if result.is_err() || stale {
            configure(gfx, size.width, size.height);
        }
        if self.app.assets().unwrap_or(&mut gfx.assets).take_overflow() {
            // The atlases were cleared mid-frame; draw again with fresh glyphs.
            self.scheduler.request_redraw();
        }
        if let Some(h) = self.app.ime() {
            let want = h.wants_ime();
            if want != *ime_allowed {
                window.set_ime_allowed(want);
                *ime_allowed = want;
            }
            if let Some(a) = h.cursor_area().filter(|_| want) {
                window.set_ime_cursor_area(
                    LogicalPosition::new(a.x, a.y),
                    LogicalSize::new(a.width, a.height),
                );
            }
        }
    }
}

impl<A: UiApp> ApplicationHandler<UserEvent> for Host<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.live.is_some() {
            return;
        }
        match self.create(event_loop) {
            Ok(mut live) => {
                {
                    let Live {
                        window,
                        adapter,
                        gfx,
                        scheme,
                        metrics,
                        ..
                    } = &mut live;
                    let mut cx = make_cx(window, adapter, &mut self.scheduler, *scheme, *metrics);
                    self.app.init(&mut cx, &mut gfx.assets);
                }
                self.scheduler.request_redraw();
                self.live = Some(live);
            }
            Err(e) => self.fail(event_loop, e),
        }
    }

    fn new_events(&mut self, _event_loop: &ActiveEventLoop, _cause: StartCause) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        let now = Instant::now();
        if self.scheduler.take_due_wake(now) {
            let mut cx = make_cx(
                &live.window,
                &mut live.adapter,
                &mut self.scheduler,
                live.scheme,
                live.metrics,
            );
            cx.now = now;
            self.app.event(&mut cx, &UiEvent::Tick(now));
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        live.adapter.process_event(&live.window, &event);
        match &event {
            WindowEvent::RedrawRequested => {
                self.redraw();
                return;
            }
            WindowEvent::CloseRequested => {
                let mut cx = make_cx(
                    &live.window,
                    &mut live.adapter,
                    &mut self.scheduler,
                    live.scheme,
                    live.metrics,
                );
                if self.app.close_requested(&mut cx) {
                    event_loop.exit();
                }
                return;
            }
            WindowEvent::Resized(size) => {
                if size.width > 0 && size.height > 0 {
                    configure(&mut live.gfx, size.width, size.height);
                }
                self.scheduler.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { .. } => self.scheduler.request_redraw(),
            WindowEvent::Focused(true) => {
                // winit gives no hook for WM_SETTINGCHANGE, so caret and contrast settings are
                // re-read whenever the window gains focus (and on theme changes below).
                live.metrics = SystemMetrics::read();
                if let Some(t) = live.window.theme() {
                    live.scheme = ColorScheme::from(t);
                }
            }
            WindowEvent::ThemeChanged(theme) => {
                live.scheme = ColorScheme::from(*theme);
                live.metrics = SystemMetrics::read();
                let scheme = live.scheme;
                let mut cx = make_cx(
                    &live.window,
                    &mut live.adapter,
                    &mut self.scheduler,
                    scheme,
                    live.metrics,
                );
                self.app.event(&mut cx, &UiEvent::Theme(scheme));
                self.scheduler.request_redraw();
                return;
            }
            WindowEvent::Ime(e) => {
                if let Some(h) = self.app.ime() {
                    ime::dispatch(h, &ime::translate(e));
                }
                self.scheduler.request_redraw();
                return;
            }
            _ => {}
        }
        let mut cx = make_cx(
            &live.window,
            &mut live.adapter,
            &mut self.scheduler,
            live.scheme,
            live.metrics,
        );
        self.app.event(&mut cx, &UiEvent::Window(&event));
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        let UserEvent::AccessKit(event) = event;
        let mut cx = make_cx(
            &live.window,
            &mut live.adapter,
            &mut self.scheduler,
            live.scheme,
            live.metrics,
        );
        match event.window_event {
            AccessEvent::InitialTreeRequested => {
                if let Some(tree) = self.app.accessibility_tree(&mut cx) {
                    cx.adapter.update_if_active(|| tree);
                }
            }
            AccessEvent::ActionRequested(request) => {
                self.app.accessibility_action(&mut cx, request);
                self.scheduler.request_redraw();
            }
            AccessEvent::AccessibilityDeactivated => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(live) = self.live.as_ref() else {
            return;
        };
        if self.scheduler.take_exit() {
            event_loop.exit();
            return;
        }
        if self.scheduler.take_redraw() {
            live.window.request_redraw();
        }
        event_loop.set_control_flow(match self.scheduler.next_wake() {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        });
    }
}
