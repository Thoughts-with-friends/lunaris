//! Drawing the core's OpenGL output.
//!
//! With the software rasteriser the core hands over CPU pixels, which become an
//! ordinary egui texture (see `app::to_image`). The OpenGL renderer instead
//! leaves the picture in a `GL_TEXTURE_2D_ARRAY` of its own — layer 0 the top
//! screen, layer 1 the bottom — at the internal resolution, and never fills the
//! CPU framebuffers at all.
//!
//! So that picture has to be drawn with GL directly, through an egui paint
//! callback that runs inside eframe's own context. This module owns the one
//! shader and vertex array that needs.
//!
//! # Why this shares eframe's context rather than making its own
//!
//! A texture belongs to the context that created it. The core renders into
//! eframe's context — that is why `Nds::run_frame` has to be called with it
//! current — so drawing here needs no sharing, no second context and no
//! readback. The cost is that the core and egui take turns with the GL state,
//! which is what [`Screen::paint`] restores.

use std::sync::Arc;

use eframe::glow::{self, HasContext};

/// The quad shader: one triangle strip covering the destination rectangle,
/// sampling one layer of the core's array texture.
const VERTEX: &str = r#"#version 330 core
out vec2 uv;
uniform vec4 rect;      // x, y, w, h in clip space
uniform vec4 uv_rect;   // u0, v0, du, dv
void main() {
    // Corners of a triangle strip, from gl_VertexID alone.
    vec2 corner = vec2(float(gl_VertexID & 1), float((gl_VertexID >> 1) & 1));
    uv = uv_rect.xy + corner * uv_rect.zw;
    gl_Position = vec4(rect.xy + corner * rect.zw, 0.0, 1.0);
}"#;

const FRAGMENT: &str = r#"#version 330 core
in vec2 uv;
out vec4 colour;
uniform sampler2DArray screen;
uniform float layer;
void main() {
    // The GL renderer's own final pass writes RGBA -- unlike the software
    // rasteriser, whose host framebuffers are BGRA and are swizzled on the CPU
    // (see `app::to_image`). Sampling it in any other order turns the DS's blue
    // sky orange, which is what this used to do.
    vec4 texel = texture(screen, vec3(uv, layer));
    colour = vec4(texel.rgb, 1.0);
}"#;

/// The GL objects needed to blit the core's output, built once.
pub struct Screen {
    program: glow::Program,
    vao: glow::VertexArray,
}

impl Screen {
    /// Compile the shader. Returns the driver's message on failure rather than
    /// panicking: a machine whose GL is too old should fall back to software,
    /// not die.
    pub fn new(gl: &glow::Context) -> Result<Self, String> {
        unsafe {
            let program = gl.create_program()?;
            let mut shaders = Vec::new();
            for (kind, source) in [(glow::VERTEX_SHADER, VERTEX), (glow::FRAGMENT_SHADER, FRAGMENT)]
            {
                let shader = gl.create_shader(kind)?;
                gl.shader_source(shader, source);
                gl.compile_shader(shader);
                if !gl.get_shader_compile_status(shader) {
                    return Err(gl.get_shader_info_log(shader));
                }
                gl.attach_shader(program, shader);
                shaders.push(shader);
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                return Err(gl.get_program_info_log(program));
            }
            // Attached shaders are no longer needed once linked.
            for shader in shaders {
                gl.detach_shader(program, shader);
                gl.delete_shader(shader);
            }

            // Core profile forbids drawing with no vertex array bound, even when
            // every vertex is computed from gl_VertexID.
            let vao = gl.create_vertex_array()?;
            Ok(Self { program, vao })
        }
    }

    /// Draw one screen of the core's array texture into `rect`.
    ///
    /// `rect` is in normalised device coordinates and `layer` selects the
    /// screen. `flip_v` accounts for GL's bottom-left origin against egui's
    /// top-left one.
    pub fn paint(&self, gl: &glow::Context, texture: u32, rect: [f32; 4], layer: f32, filter: u32) {
        unsafe {
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vao));

            let texture = glow::NativeTexture(std::num::NonZeroU32::new(texture).unwrap());
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(texture));
            gl.tex_parameter_i32(glow::TEXTURE_2D_ARRAY, glow::TEXTURE_MIN_FILTER, filter as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D_ARRAY, glow::TEXTURE_MAG_FILTER, filter as i32);
            gl.tex_parameter_i32(
                glow::TEXTURE_2D_ARRAY,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D_ARRAY,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );

            set_uniform4(gl, self.program, "rect", rect);
            // Flipped vertically: the core's texture has GL's origin.
            set_uniform4(gl, self.program, "uv_rect", [0.0, 1.0, 1.0, -1.0]);
            if let Some(loc) = gl.get_uniform_location(self.program, "layer") {
                gl.uniform_1_f32(Some(&loc), layer);
            }
            if let Some(loc) = gl.get_uniform_location(self.program, "screen") {
                gl.uniform_1_i32(Some(&loc), 0);
            }

            // egui composites with blending on; the console's picture is opaque
            // and must not be blended against whatever is underneath.
            gl.disable(glow::BLEND);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::SCISSOR_TEST);
            // The core's texels are already the values the DS displays, so the
            // driver must not encode them again on the way to the framebuffer.
            // Measured off in eframe's context here (egui encodes in its own
            // shader instead), which makes this a guard rather than a fix: it
            // costs one query and keeps the picture right on a driver that
            // does enable it. Restored either way, since egui's painter is
            // entitled to whatever state it set.
            let srgb = gl.is_enabled(glow::FRAMEBUFFER_SRGB);
            if srgb {
                gl.disable(glow::FRAMEBUFFER_SRGB);
            }
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            if srgb {
                gl.enable(glow::FRAMEBUFFER_SRGB);
            }

            // Put back what egui's painter expects to find.
            gl.enable(glow::BLEND);
            gl.bind_vertex_array(None);
            gl.use_program(None);
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
            gl.delete_vertex_array(self.vao);
        }
    }
}

unsafe fn set_uniform4(gl: &glow::Context, program: glow::Program, name: &str, v: [f32; 4]) {
    unsafe {
        if let Some(loc) = gl.get_uniform_location(program, name) {
            gl.uniform_4_f32(Some(&loc), v[0], v[1], v[2], v[3]);
        }
    }
}

/// A [`Screen`] shared with the paint callbacks, which egui may run after the
/// frame that queued them.
pub type Shared = Arc<Screen>;
