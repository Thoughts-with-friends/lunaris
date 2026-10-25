//! An OpenGL context of the emulator thread's own, for melonDS's OpenGL
//! renderer (internal resolution above 1x).
//!
//! melon_egui borrows eframe's context; freya draws with Skia and keeps its
//! context to itself, so this makes one: a hidden 1x1 window (the `STATIC`
//! class, nothing to register) and a WGL context on it, current on the
//! thread that runs the console. melonDS renders into its own framebuffers;
//! the window only carries the context.
//!
//! ```text
//!  emulator thread: GlContext::new() -> melonds::gl_load(None)
//!     -> Nds::set_render_settings(OpenGl, scale) -> run_frame
//!     -> Nds::gl_read_output(screen) at 256n x 192n
//! ```
#![cfg(windows)]

use std::num::{NonZeroIsize, NonZeroU32};

use glutin::{
    config::ConfigTemplateBuilder,
    context::{
        ContextApi, ContextAttributesBuilder, GlProfile, NotCurrentGlContext as _,
        PossiblyCurrentContext, Version,
    },
    display::{Display, DisplayApiPreference, GlDisplay as _},
    surface::{Surface, SurfaceAttributesBuilder, WindowSurface},
};
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle,
};
use windows_sys::Win32::{
    Foundation::HWND,
    System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WS_POPUP},
};

/// A current GL context, dropped (and the window destroyed) with the core.
pub struct GlContext {
    // Field order is drop order: context, surface, display, then the window.
    _context: PossiblyCurrentContext,
    _surface: Surface<WindowSurface>,
    _display: Display,
    hwnd: HWND,
}

impl Drop for GlContext {
    fn drop(&mut self) {
        // SAFETY: the window was made by this thread in `new` and is not
        // used after this.
        unsafe { DestroyWindow(self.hwnd) };
    }
}

impl GlContext {
    /// Makes the context and makes it current on this thread.
    ///
    /// # Errors
    /// If this PC has no usable OpenGL 3.2+ driver.
    pub fn new() -> Result<Self, String> {
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        // SAFETY: a hidden child-less popup with a system class; checked below.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                std::ptr::null(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        let handle = NonZeroIsize::new(hwnd as isize).ok_or("Cannot create the GL window")?;
        let mut win32 = Win32WindowHandle::new(handle);
        // glutin's WGL loader needs the module too.
        // SAFETY: the handle of this executable, no ownership taken.
        win32.hinstance = NonZeroIsize::new(unsafe { GetModuleHandleW(std::ptr::null()) } as isize);
        let window = RawWindowHandle::Win32(win32);
        let made = Self::on_window(window).map_err(|e| format!("OpenGL: {e}"));
        match made {
            Ok((context, surface, display)) => {
                Ok(Self { _context: context, _surface: surface, _display: display, hwnd })
            }
            Err(e) => {
                // SAFETY: as in `Drop`.
                unsafe { DestroyWindow(hwnd) };
                Err(e)
            }
        }
    }

    fn on_window(
        window: RawWindowHandle,
    ) -> Result<(PossiblyCurrentContext, Surface<WindowSurface>, Display), glutin::error::Error>
    {
        // SAFETY: the window lives until the context is dropped (`GlContext`).
        let display = unsafe {
            Display::new(
                RawDisplayHandle::Windows(WindowsDisplayHandle::new()),
                DisplayApiPreference::Wgl(Some(window)),
            )
        }?;
        let template = ConfigTemplateBuilder::new().compatible_with_native_window(window).build();
        // SAFETY: as above.
        let config = unsafe { display.find_configs(template) }?
            .next()
            .ok_or(glutin::error::ErrorKind::NotFound)?;
        // The compute renderer needs 4.3; the regular one 3.2. Newest first.
        let mut context = None;
        for version in [Version::new(4, 3), Version::new(3, 2)] {
            let attributes = ContextAttributesBuilder::new()
                .with_profile(GlProfile::Core)
                .with_context_api(ContextApi::OpenGl(Some(version)))
                .build(Some(window));
            // SAFETY: as above.
            if let Ok(made) = unsafe { display.create_context(&config, &attributes) } {
                context = Some(made);
                break;
            }
        }
        let context = context.ok_or(glutin::error::ErrorKind::NotSupported("OpenGL 3.2"))?;
        let one = NonZeroU32::MIN;
        let surface_attributes =
            SurfaceAttributesBuilder::<WindowSurface>::new().build(window, one, one);
        // SAFETY: as above.
        let surface = unsafe { display.create_window_surface(&config, &surface_attributes) }?;
        let context = context.make_current(&surface)?;
        Ok((context, surface, display))
    }
}
