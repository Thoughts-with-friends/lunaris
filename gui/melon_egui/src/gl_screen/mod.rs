//! Drawing the core's OpenGL output.
//!
//! With the software rasteriser the core hands over CPU pixels, which become an
//! ordinary egui texture (see `app::to_image`). The OpenGL renderer instead
//! leaves the picture in a `GL_TEXTURE_2D_ARRAY` of its own — layer 0 the top
//! screen, layer 1 the bottom — at the internal resolution, and never fills the
//! CPU framebuffers at all.
//!
//! So that picture has to be drawn with GL directly, through an egui paint
//! callback that runs inside eframe's own context. This module owns the shader,
//! the vertex array and the frame buffer that needs.
//!
//! # Why this shares eframe's context rather than making its own
//!
//! A texture belongs to the context that created it. The core renders into
//! eframe's context — that is why `Nds::run_frame` has to be called with it
//! current — so drawing here needs no sharing and no second context. The cost
//! is that the core and egui take turns with the GL state, which is what
//! [`Screen::paint`] restores.
//!
//! # xBRZ under this renderer
//!
//! [`shader`] explains what is drawn and [`capture`] how the 2D content makes
//! its round trip through the CPU filter. The short of it: the 2D is put
//! through the *same* xBRZ the software renderer uses, at the same 256x192, and
//! the 3D keeps every pixel the GPU drew.

use std::sync::Arc;

use eframe::glow::{self, HasContext};

pub mod capture;
pub mod shader;

#[cfg(test)]
mod rule;

pub use capture::{DS_HEIGHT, DS_WIDTH};

/// The GL objects needed to blit the core's output, built once.
pub struct Screen {
    program: glow::Program,
    vao: glow::VertexArray,
    /// The 2D round trip. `None` when the driver would not give us a frame
    /// buffer, which costs xBRZ under this renderer and nothing else.
    capture: Option<capture::Capture>,
}

impl Screen {
    /// Compile the shader. Returns the driver's message on failure rather than
    /// panicking: a machine whose GL is too old should fall back to software,
    /// not die.
    ///
    /// # Errors
    /// If the shader will not build or the vertex array cannot be made.
    pub fn new(gl: &glow::Context) -> Result<Self, String> {
        unsafe {
            let program = gl.create_program()?;
            let mut shaders = Vec::new();
            for (kind, source) in
                [(glow::VERTEX_SHADER, shader::VERTEX), (glow::FRAGMENT_SHADER, shader::FRAGMENT)]
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
            // Reported rather than fatal: without it the picture still draws,
            // it just cannot have xBRZ on its 2D.
            let capture = capture::Capture::new(gl)
                .inspect_err(|error| {
                    eprintln!("melon_egui: no 2D capture ({error}); xBRZ is off under OpenGL");
                })
                .ok();
            Ok(Self { program, vao, capture })
        }
    }

    /// Whether xBRZ can run under this renderer at all.
    #[must_use]
    pub const fn can_filter(&self) -> bool {
        self.capture.is_some()
    }

    /// Take one screen's 2D content back off the GPU, at the DS's own size.
    ///
    /// Returns `DS_WIDTH * DS_HEIGHT` RGBA bytes, ready for
    /// `crate::upscale::upscale`. `None` when there is no capture target.
    ///
    /// Must be called with eframe's context current — which is to say from
    /// `App::update`, not from a paint callback.
    pub fn read_ds_pixels(&self, gl: &glow::Context, texture: u32, layer: u32) -> Option<Vec<u8>> {
        let capture = self.capture.as_ref()?;
        Some(capture.read_ds_pixels(gl, || {
            // No flip: the bytes stay in the array texture's own order, which
            // is the order the shader samples the filtered result back in.
            self.draw(
                gl,
                texture,
                Pass {
                    rect: FULL_CLIP,
                    uv_rect: [0.0, 0.0, 1.0, 1.0],
                    layer: layer as f32,
                    filter: glow::NEAREST,
                    use_filtered: 0,
                },
            );
        }))
    }

    /// Hand one screen its filtered picture.
    pub fn write_filtered(
        &self,
        gl: &glow::Context,
        layer: u32,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) {
        if let Some(capture) = &self.capture {
            capture.write_filtered(gl, layer as usize, rgba, width, height);
        }
    }

    /// Forget both screens' filtered pictures, so a stale one is never shown.
    pub fn invalidate_filtered(&self) {
        if let Some(capture) = &self.capture {
            capture.invalidate();
        }
    }

    /// Draw one screen of the core's array texture into `rect`.
    ///
    /// `rect` is in normalised device coordinates and `layer` selects the
    /// screen. `filtered` asks for the xBRZ'd 2D, which is only shown where one
    /// has been supplied and the fragment's DS pixel is a uniform block — see
    /// [`shader`].
    pub fn paint(
        &self,
        gl: &glow::Context,
        texture: u32,
        rect: [f32; 4],
        layer: f32,
        filter: u32,
        filtered: bool,
    ) {
        let ready =
            filtered && self.capture.as_ref().is_some_and(|c| c.has_filtered(layer as usize));
        unsafe {
            if ready && let Some(capture) = &self.capture {
                gl.active_texture(glow::TEXTURE1);
                gl.bind_texture(glow::TEXTURE_2D, Some(capture.filtered(layer as usize)));
            }
            // Flipped vertically: the core's texture has GL's origin, and egui
            // has the other one.
            self.draw(
                gl,
                texture,
                Pass {
                    rect,
                    uv_rect: [0.0, 1.0, 1.0, -1.0],
                    layer,
                    filter,
                    use_filtered: u32::from(ready),
                },
            );
        }
    }

    /// One draw of the quad. Everything both callers share.
    fn draw(&self, gl: &glow::Context, texture: u32, pass: Pass) {
        let Pass { rect, uv_rect, layer, filter, use_filtered } = pass;
        let Some(texture) = std::num::NonZeroU32::new(texture).map(glow::NativeTexture) else {
            return;
        };
        unsafe {
            gl.use_program(Some(self.program));
            gl.bind_vertex_array(Some(self.vao));

            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D_ARRAY, Some(texture));
            // Left as the caller asked: the block test reads exact texels
            // through `texelFetch` regardless, and `Screen filtering` decides
            // how the 3D — which the shader passes through — is magnified.
            for name in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] {
                gl.tex_parameter_i32(glow::TEXTURE_2D_ARRAY, name, filter as i32);
            }
            for name in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] {
                gl.tex_parameter_i32(glow::TEXTURE_2D_ARRAY, name, glow::CLAMP_TO_EDGE as i32);
            }

            set_uniform4(gl, self.program, "rect", rect);
            set_uniform4(gl, self.program, "uv_rect", uv_rect);
            set_uniform1i(gl, self.program, "screen", 0);
            set_uniform1i(gl, self.program, "filtered", 1);
            set_uniform1i(gl, self.program, "use_filtered", use_filtered as i32);
            if let Some(loc) = gl.get_uniform_location(self.program, "layer") {
                gl.uniform_1_f32(Some(&loc), layer);
            }

            // egui composites with blending on; the console's picture is opaque
            // and must not be blended against whatever is underneath.
            gl.disable(glow::BLEND);
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::SCISSOR_TEST);
            // The core's texels are already the values the DS displays, so the
            // driver must not encode them again on the way to the framebuffer.
            // Measured off in eframe's context here (egui encodes in its own
            // shader instead), which makes this a guard rather than a fix.
            // Restored either way, since egui's painter is entitled to whatever
            // state it set.
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
            gl.active_texture(glow::TEXTURE0);
            gl.bind_vertex_array(None);
            gl.use_program(None);
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
            gl.delete_vertex_array(self.vao);
        }
        if let Some(capture) = &self.capture {
            capture.destroy(gl);
        }
    }
}

/// What one draw of the quad needs.
///
/// A struct rather than a long argument list: the two callers differ in every
/// field, and five bare values in a row is exactly the shape a transposition
/// hides in.
struct Pass {
    /// Where to draw, in normalised device coordinates.
    rect: [f32; 4],
    /// Which part of the source to read, and which way up.
    uv_rect: [f32; 4],
    /// Which screen.
    layer: f32,
    /// How the source is magnified.
    filter: u32,
    /// Whether to show the xBRZ'd 2D where the shader finds it.
    use_filtered: u32,
}

/// A quad covering the whole GL viewport, which egui_glow has already set to
/// the paint callback's rectangle.
pub const FULL_CLIP: [f32; 4] = [-1.0, -1.0, 2.0, 2.0];

unsafe fn set_uniform4(gl: &glow::Context, program: glow::Program, name: &str, v: [f32; 4]) {
    unsafe {
        if let Some(loc) = gl.get_uniform_location(program, name) {
            gl.uniform_4_f32(Some(&loc), v[0], v[1], v[2], v[3]);
        }
    }
}

unsafe fn set_uniform1i(gl: &glow::Context, program: glow::Program, name: &str, v: i32) {
    unsafe {
        if let Some(loc) = gl.get_uniform_location(program, name) {
            gl.uniform_1_i32(Some(&loc), v);
        }
    }
}

/// A [`Screen`] shared with the paint callbacks, which egui may run after the
/// frame that queued them.
pub type Shared = Arc<Screen>;
