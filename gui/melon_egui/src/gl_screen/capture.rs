//! Getting the 2D content off the GPU, and the filtered result back on.
//!
//! # What is read back, and why it is cheap
//!
//! Not the frame. One texel per **DS pixel** — 256x192 whatever the internal
//! resolution is, because that is where the 2D content actually lives (see
//! [`super::shader`]). A trivial GPU pass draws the screen into a 256x192
//! target with nearest sampling, so each destination texel takes the middle of
//! one block, and that is what is read.
//!
//! 196 KB per screen per frame, against 3 MB at 4x internal if the whole frame
//! came back — and, far more importantly, xBRZ then runs over 49 152 pixels
//! instead of 786 432. That is the same amount of work the software renderer
//! already does, which is what makes "as good as software" affordable at all.
//!
//! # What is sent back
//!
//! The filtered image, as a plain 2D texture per screen. The blit shader
//! samples it wherever the DS pixel under a fragment was a uniform block, and
//! the renderer's own texture everywhere else.
//!
//! Orientation is deliberately left alone throughout: the capture pass does not
//! flip, so the bytes are in the array texture's own order, and the shader
//! samples both with the same coordinates. xBRZ is symmetric under reflection,
//! so filtering upside down and filtering right way up give the same picture.

use std::sync::atomic::{AtomicU32, Ordering};

use eframe::glow::{self, HasContext};

/// The DS's own screen: the size the 2D content is read back at.
pub const DS_WIDTH: u32 = 256;
/// See [`DS_WIDTH`].
pub const DS_HEIGHT: u32 = 192;

/// The GPU objects the capture and the filtered result need.
///
/// Built once with the blitter, because a frame buffer object and two textures
/// are cheap to hold and expensive to keep creating.
pub struct Capture {
    /// Where the downsample pass draws.
    framebuffer: glow::Framebuffer,
    /// Its colour attachment, 256x192.
    target: glow::Texture,
    /// The filtered picture for each screen.
    filtered: [glow::Texture; 2],
    /// What size each of those currently holds, packed `width << 16 | height`,
    /// so the storage is only reallocated when the factor changes.
    ///
    /// Atomic because the blitter is shared with egui's paint callbacks, which
    /// require it to be `Sync`.
    sizes: [AtomicU32; 2],
}

impl Capture {
    /// Build the frame buffer and the textures.
    ///
    /// # Errors
    /// If the driver will not create them.
    pub fn new(gl: &glow::Context) -> Result<Self, String> {
        unsafe {
            let target = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(target));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                DS_WIDTH as i32,
                DS_HEIGHT as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            set_clamped(gl, glow::NEAREST);

            let framebuffer = gl.create_framebuffer()?;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(target),
                0,
            );
            let complete = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            if complete != glow::FRAMEBUFFER_COMPLETE {
                return Err(format!("capture frame buffer is not complete ({complete:#x})"));
            }

            let filtered = [gl.create_texture()?, gl.create_texture()?];
            for texture in filtered {
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                set_clamped(gl, glow::LINEAR);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);

            Ok(Self {
                framebuffer,
                target,
                filtered,
                sizes: [AtomicU32::new(0), AtomicU32::new(0)],
            })
        }
    }

    /// The texture holding `layer`'s filtered picture.
    pub fn filtered(&self, layer: usize) -> glow::Texture {
        self.filtered[layer & 1]
    }

    /// Whether `layer` has been given a filtered picture yet.
    pub fn has_filtered(&self, layer: usize) -> bool {
        self.sizes[layer & 1].load(Ordering::Relaxed) != 0
    }

    /// Draw one screen into the 256x192 target and read it back as RGBA.
    ///
    /// `draw` is [`super::Screen`]'s own blit, called with the target bound and
    /// the viewport set; keeping it a closure is what stops this module needing
    /// a second copy of the shader.
    ///
    /// Restores the frame buffer and the viewport, because egui's painter is
    /// entitled to find them as it left them.
    pub fn read_ds_pixels(&self, gl: &glow::Context, draw: impl FnOnce()) -> Vec<u8> {
        let mut pixels = vec![0u8; (DS_WIDTH * DS_HEIGHT * 4) as usize];
        unsafe {
            let previous = gl.get_parameter_i32(glow::FRAMEBUFFER_BINDING);
            let mut viewport = [0i32; 4];
            gl.get_parameter_i32_slice(glow::VIEWPORT, &mut viewport);

            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(self.framebuffer));
            gl.viewport(0, 0, DS_WIDTH as i32, DS_HEIGHT as i32);
            draw();
            gl.read_pixels(
                0,
                0,
                DS_WIDTH as i32,
                DS_HEIGHT as i32,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelPackData::Slice(Some(&mut pixels)),
            );

            gl.bind_framebuffer(
                glow::FRAMEBUFFER,
                std::num::NonZeroU32::new(previous as u32).map(glow::NativeFramebuffer),
            );
            gl.viewport(viewport[0], viewport[1], viewport[2], viewport[3]);
        }
        pixels
    }

    /// Hand `layer` its filtered picture, `width` by `height` RGBA.
    pub fn write_filtered(
        &self,
        gl: &glow::Context,
        layer: usize,
        rgba: &[u8],
        width: u32,
        height: u32,
    ) {
        let expected = (width as usize) * (height as usize) * 4;
        if rgba.len() != expected || width == 0 || height == 0 {
            return;
        }
        let packed = (width << 16) | height;
        unsafe {
            gl.bind_texture(glow::TEXTURE_2D, Some(self.filtered[layer & 1]));
            // Reallocated only when the factor changed; otherwise the existing
            // storage is written over, which is what keeps this off the
            // allocator sixty times a second.
            if self.sizes[layer & 1].swap(packed, Ordering::Relaxed) == packed {
                gl.tex_sub_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    0,
                    0,
                    width as i32,
                    height as i32,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(rgba)),
                );
            } else {
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    width as i32,
                    height as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(rgba)),
                );
                set_clamped(gl, glow::LINEAR);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
    }

    /// Forget that either screen has a filtered picture.
    ///
    /// Called when the filter is switched off or the factor changes, so that a
    /// stale picture cannot be shown for the frame before the new one arrives.
    pub fn invalidate(&self) {
        for size in &self.sizes {
            size.store(0, Ordering::Relaxed);
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_framebuffer(self.framebuffer);
            gl.delete_texture(self.target);
            for texture in self.filtered {
                gl.delete_texture(texture);
            }
        }
    }
}

/// The wrapping and filtering every texture here wants.
unsafe fn set_clamped(gl: &glow::Context, filter: u32) {
    unsafe {
        for (name, value) in [
            (glow::TEXTURE_MIN_FILTER, filter as i32),
            (glow::TEXTURE_MAG_FILTER, filter as i32),
            (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32),
            (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, name, value);
        }
    }
}
