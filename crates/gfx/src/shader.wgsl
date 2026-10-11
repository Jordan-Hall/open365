// Open365 2D renderer. Vertices are in device pixels; colours arrive as sRGB bytes and are
// blended in linear light (the render target is an *Srgb format, so the hardware encodes on
// write and decodes on blend). Modes match `draw::mode`.

struct Globals {
    viewport: vec2<f32>,
    atlas_size: f32,
    // 1.0 when the target is not an sRGB format: encode in the shader (blending is then not
    // gamma-correct; only a fallback).
    srgb_encode: f32,
    text_gamma: f32,
    text_contrast: f32,
    // The same two controls for light ink (white text on a dark background).
    text_gamma_light: f32,
    text_contrast_light: f32,
}

@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var mask_tex: texture_2d<f32>;
@group(1) @binding(1) var lcd_tex: texture_2d<f32>;
@group(1) @binding(2) var color_tex: texture_2d<f32>;
@group(1) @binding(3) var linear_samp: sampler;

// Coverage mask of a rounded or path clip, bound per batch with a dynamic offset. `enabled` is 0
// when the batch has no mask.
struct ClipU {
    mask_origin: vec2<f32>,
    dev_origin: vec2<f32>,
    size: vec2<f32>,
    enabled: f32,
    pad: f32,
}
@group(2) @binding(0) var<uniform> clip_u: ClipU;

struct VertexIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) mode: u32,
    @location(4) params: vec4<f32>,
}

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) mode: u32,
    @location(3) @interpolate(flat) params: vec4<f32>,
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = vec4<f32>(
        in.pos.x / globals.viewport.x * 2.0 - 1.0,
        1.0 - in.pos.y / globals.viewport.y * 2.0,
        0.0,
        1.0,
    );
    out.uv = in.uv;
    // Gradients interpolate in sRGB; the fragment stage converts.
    var rgb = in.color.rgb;
    if in.mode != 6u {
        rgb = srgb_to_linear(rgb);
    }
    out.color = vec4<f32>(rgb, in.color.a);
    out.mode = in.mode;
    out.params = in.params;
    return out;
}

// Signed distance to a rounded box centred on the origin with half-extents `b`.
fn sd_round_box(p: vec2<f32>, b: vec2<f32>, r: f32) -> f32 {
    let q = abs(p) - b + vec2<f32>(r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

// Text coverage shaping. Identity at the defaults (gamma 1, contrast 0). Dark text on a light
// background looks thin when blended in linear light, so gamma > 1 and contrast > 0 thicken
// dark ink and leave light ink alone. The values are to be fitted against DirectWrite captures.
fn shape_coverage(cov: f32, ink: vec3<f32>) -> f32 {
    let lum = dot(ink, vec3<f32>(0.2126, 0.7152, 0.0722));
    let darkness = 1.0 - clamp(lum, 0.0, 1.0);
    let g = mix(globals.text_gamma_light, globals.text_gamma, darkness);
    let k = mix(globals.text_contrast_light, globals.text_contrast, darkness);
    var c = pow(clamp(cov, 0.0, 1.0), 1.0 / g);
    c = c + k * c * (1.0 - c);
    return clamp(c, 0.0, 1.0);
}

fn clip_coverage(frag: vec2<f32>) -> f32 {
    if clip_u.enabled < 0.5 {
        return 1.0;
    }
    let p = floor(frag) - clip_u.dev_origin;
    if p.x < 0.0 || p.y < 0.0 || p.x >= clip_u.size.x || p.y >= clip_u.size.y {
        return 0.0;
    }
    return textureLoad(mask_tex, vec2<i32>(clip_u.mask_origin + p), 0).r;
}

// Gaussian-blurred edge: coverage of a half-plane at signed distance d (negative inside).
fn blur_edge(d: f32, sigma: f32) -> f32 {
    let t = d / sigma;
    return 0.5 - 0.5 * tanh(0.7978845608 * (t + 0.044715 * t * t * t));
}

fn finish(rgb: vec3<f32>, a: f32) -> vec4<f32> {
    // Premultiplied output.
    var out_rgb = rgb;
    if globals.srgb_encode > 0.5 {
        out_rgb = linear_to_srgb(rgb);
    }
    return vec4<f32>(out_rgb * a, a);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    var cov = 1.0;
    var color = in.color;
    switch in.mode {
        case 1u: {
            let d = sd_round_box(in.uv, in.params.xy, in.params.z);
            cov = clamp(0.5 - d, 0.0, 1.0);
            if in.params.w > 0.0 {
                cov = cov * clamp(d + in.params.w + 0.5, 0.0, 1.0);
            }
        }
        case 2u: {
            let t = textureSampleLevel(color_tex, linear_samp, in.uv / globals.atlas_size, 0.0);
            color = vec4<f32>(color.rgb * t.rgb, color.a * t.a);
        }
        case 3u: {
            let c = textureLoad(mask_tex, vec2<i32>(in.uv), 0).r;
            cov = shape_coverage(c, in.color.rgb);
        }
        case 5u: {
            cov = textureLoad(mask_tex, vec2<i32>(in.uv), 0).r;
        }
        case 6u: {
            color = vec4<f32>(srgb_to_linear(in.color.rgb), in.color.a);
        }
        case 7u: {
            let d = sd_round_box(in.uv, in.params.xy, in.params.z);
            cov = blur_edge(d, in.params.w);
        }
        default: {}
    }
    return finish(color.rgb, color.a * cov * clip_coverage(in.clip.xy));
}

// LCD text: drawn three times, once per colour channel (write mask R, G or B), with
// non-premultiplied alpha equal to that channel's coverage. No dual-source blending needed.
override LCD_CHANNEL: u32 = 0u;

@fragment
fn fs_lcd(in: VertexOut) -> @location(0) vec4<f32> {
    let t = textureLoad(lcd_tex, vec2<i32>(in.uv), 0);
    var c = t.r;
    if LCD_CHANNEL == 1u {
        c = t.g;
    } else if LCD_CHANNEL == 2u {
        c = t.b;
    }
    c = shape_coverage(c, in.color.rgb);
    var rgb = in.color.rgb;
    if globals.srgb_encode > 0.5 {
        rgb = linear_to_srgb(rgb);
    }
    return vec4<f32>(rgb, c * in.color.a * clip_coverage(in.clip.xy));
}
