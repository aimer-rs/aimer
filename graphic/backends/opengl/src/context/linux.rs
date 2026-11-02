use std::ffi::{CStr, c_char, c_void};
use std::ptr::{null, null_mut};

use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

type EglBoolean = u32;
type EglInt = i32;
type EglEnum = u32;
type EglDisplay = *mut c_void;
type EglConfig = *mut c_void;
type EglSurface = *mut c_void;
type EglContext = *mut c_void;

const EGL_FALSE: EglBoolean = 0;
const EGL_NONE: EglInt = 0x3038;
const EGL_DEFAULT_DISPLAY: *mut c_void = null_mut();
const EGL_OPENGL_API: EglEnum = 0x30A2;
const EGL_OPENGL_ES_API: EglEnum = 0x30A0;
const EGL_OPENGL_BIT: EglInt = 0x0008;
const EGL_OPENGL_ES3_BIT: EglInt = 0x0040;
const EGL_WINDOW_BIT: EglInt = 0x0004;
const EGL_PBUFFER_BIT: EglInt = 0x0001;
const EGL_RED_SIZE: EglInt = 0x3024;
const EGL_GREEN_SIZE: EglInt = 0x3023;
const EGL_BLUE_SIZE: EglInt = 0x3022;
const EGL_ALPHA_SIZE: EglInt = 0x3021;
const EGL_DEPTH_SIZE: EglInt = 0x3025;
const EGL_STENCIL_SIZE: EglInt = 0x3026;
const EGL_SURFACE_TYPE: EglInt = 0x3033;
const EGL_RENDERABLE_TYPE: EglInt = 0x3040;
const EGL_NATIVE_VISUAL_ID: EglInt = 0x302E;
const EGL_WIDTH: EglInt = 0x3057;
const EGL_HEIGHT: EglInt = 0x3056;
const EGL_CONTEXT_MAJOR_VERSION_KHR: EglInt = 0x3098;
const EGL_CONTEXT_MINOR_VERSION_KHR: EglInt = 0x30FB;
const EGL_CONTEXT_OPENGL_PROFILE_MASK_KHR: EglInt = 0x30FD;
const EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT_KHR: EglInt = 0x0001;
const EGL_CONTEXT_CLIENT_VERSION: EglInt = 0x3098;
const EGL_PLATFORM_X11_EXT: EglEnum = 0x31D5;
const EGL_PLATFORM_WAYLAND_EXT: EglEnum = 0x31D8;
const EGL_PLATFORM_XCB_EXT: EglEnum = 0x31DC;
const EGL_PLATFORM_SURFACELESS_MESA: EglEnum = 0x31DD;
const RTLD_NOW: i32 = 2;
const RTLD_LOCAL: i32 = 0;

#[link(name = "dl")]
unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> i32;
}

type EglGetDisplay = unsafe extern "C" fn(*mut c_void) -> EglDisplay;
type EglInitialize = unsafe extern "C" fn(EglDisplay, *mut EglInt, *mut EglInt) -> EglBoolean;
type EglChooseConfig = unsafe extern "C" fn(
    EglDisplay,
    *const EglInt,
    *mut EglConfig,
    EglInt,
    *mut EglInt,
) -> EglBoolean;
type EglBindApi = unsafe extern "C" fn(EglEnum) -> EglBoolean;
type EglCreateContext =
    unsafe extern "C" fn(EglDisplay, EglConfig, EglContext, *const EglInt) -> EglContext;
type EglCreateWindowSurface =
    unsafe extern "C" fn(EglDisplay, EglConfig, *mut c_void, *const EglInt) -> EglSurface;
type EglCreatePbufferSurface =
    unsafe extern "C" fn(EglDisplay, EglConfig, *const EglInt) -> EglSurface;
type EglCreatePlatformWindowSurface =
    unsafe extern "C" fn(EglDisplay, EglConfig, *mut c_void, *const EglInt) -> EglSurface;
type EglGetPlatformDisplay =
    unsafe extern "C" fn(EglEnum, *mut c_void, *const EglInt) -> EglDisplay;
type EglMakeCurrent =
    unsafe extern "C" fn(EglDisplay, EglSurface, EglSurface, EglContext) -> EglBoolean;
type EglSwapBuffers = unsafe extern "C" fn(EglDisplay, EglSurface) -> EglBoolean;
type EglSwapInterval = unsafe extern "C" fn(EglDisplay, EglInt) -> EglBoolean;
type EglDestroySurface = unsafe extern "C" fn(EglDisplay, EglSurface) -> EglBoolean;
type EglDestroyContext = unsafe extern "C" fn(EglDisplay, EglContext) -> EglBoolean;
type EglTerminate = unsafe extern "C" fn(EglDisplay) -> EglBoolean;
type EglGetError = unsafe extern "C" fn() -> EglInt;
type EglGetProcAddress = unsafe extern "C" fn(*const c_char) -> *const c_void;

type WaylandEglWindowCreate = unsafe extern "C" fn(*mut c_void, i32, i32) -> *mut c_void;
type WaylandEglWindowResize = unsafe extern "C" fn(*mut c_void, i32, i32, i32, i32);
type WaylandEglWindowDestroy = unsafe extern "C" fn(*mut c_void);

macro_rules! cstr {
    ($literal:literal) => {{
        // SAFETY: each use is a source literal without interior NUL bytes.
        unsafe { CStr::from_bytes_with_nul_unchecked(concat!($literal, "\0").as_bytes()) }
    }};
}

struct EglFns {
    library: *mut c_void,
    get_display: EglGetDisplay,
    initialize: EglInitialize,
    choose_config: EglChooseConfig,
    bind_api: EglBindApi,
    create_context: EglCreateContext,
    create_window_surface: EglCreateWindowSurface,
    create_pbuffer_surface: EglCreatePbufferSurface,
    make_current: EglMakeCurrent,
    swap_buffers: EglSwapBuffers,
    swap_interval: EglSwapInterval,
    destroy_surface: EglDestroySurface,
    destroy_context: EglDestroyContext,
    terminate: EglTerminate,
    get_error: EglGetError,
    get_proc_address: EglGetProcAddress,
    get_platform_display: Option<EglGetPlatformDisplay>,
    create_platform_window_surface: Option<EglCreatePlatformWindowSurface>,
}

impl EglFns {
    fn load() -> Result<Self, String> {
        // SAFETY: dlopen receives a static, nul-terminated system library name.
        #[cfg(target_os = "android")]
        let library_name = c"libEGL.so";
        #[cfg(not(target_os = "android"))]
        let library_name = c"libEGL.so.1";
        let library = unsafe { dlopen(library_name.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
        if library.is_null() {
            return Err(format!("could not load {}", library_name.to_string_lossy()));
        }
        macro_rules! symbol {
            ($name:literal, $ty:ty) => {{
                // SAFETY: library is live and each symbol is requested by its exact ABI name.
                let address = unsafe { dlsym(library, cstr!($name).as_ptr()) };
                if address.is_null() {
                    unsafe { dlclose(library) };
                    return Err(format!(
                        "{} is missing {}",
                        library_name.to_string_lossy(),
                        $name
                    ));
                }
                // SAFETY: the EGL implementation exports this function with the declared ABI.
                unsafe { std::mem::transmute::<*mut c_void, $ty>(address) }
            }};
        }
        macro_rules! optional_symbol {
            ($name:literal, $ty:ty) => {{
                let address = unsafe { dlsym(library, cstr!($name).as_ptr()) };
                if address.is_null() {
                    None
                } else {
                    Some(unsafe { std::mem::transmute::<*mut c_void, $ty>(address) })
                }
            }};
        }
        Ok(Self {
            library,
            get_display: symbol!("eglGetDisplay", EglGetDisplay),
            initialize: symbol!("eglInitialize", EglInitialize),
            choose_config: symbol!("eglChooseConfig", EglChooseConfig),
            bind_api: symbol!("eglBindAPI", EglBindApi),
            create_context: symbol!("eglCreateContext", EglCreateContext),
            create_window_surface: symbol!("eglCreateWindowSurface", EglCreateWindowSurface),
            create_pbuffer_surface: symbol!("eglCreatePbufferSurface", EglCreatePbufferSurface),
            make_current: symbol!("eglMakeCurrent", EglMakeCurrent),
            swap_buffers: symbol!("eglSwapBuffers", EglSwapBuffers),
            swap_interval: symbol!("eglSwapInterval", EglSwapInterval),
            destroy_surface: symbol!("eglDestroySurface", EglDestroySurface),
            destroy_context: symbol!("eglDestroyContext", EglDestroyContext),
            terminate: symbol!("eglTerminate", EglTerminate),
            get_error: symbol!("eglGetError", EglGetError),
            get_proc_address: symbol!("eglGetProcAddress", EglGetProcAddress),
            get_platform_display: optional_symbol!("eglGetPlatformDisplay", EglGetPlatformDisplay),
            create_platform_window_surface: optional_symbol!(
                "eglCreatePlatformWindowSurface",
                EglCreatePlatformWindowSurface
            ),
        })
    }

    unsafe fn proc(&self, name: &CStr) -> *const c_void {
        // SAFETY: EGL accepts any nul-terminated function name and returns null
        // when the implementation does not expose it.
        let address = unsafe { (self.get_proc_address)(name.as_ptr()) };
        if !address.is_null() {
            return address;
        }
        // SAFETY: library remains open for the function table's lifetime.
        unsafe { dlsym(self.library, name.as_ptr()) }
    }
}

impl Drop for EglFns {
    fn drop(&mut self) {
        unsafe { dlclose(self.library) };
    }
}

struct WaylandEgl {
    library: *mut c_void,
    window: *mut c_void,
    resize: WaylandEglWindowResize,
    destroy: WaylandEglWindowDestroy,
}

impl WaylandEgl {
    fn new(surface: *mut c_void, size: (u32, u32)) -> Result<Self, String> {
        let library = unsafe { dlopen(c"libwayland-egl.so.1".as_ptr(), RTLD_NOW | RTLD_LOCAL) };
        if library.is_null() {
            return Err("could not load libwayland-egl.so.1".to_owned());
        }
        let create = unsafe { dlsym(library, c"wl_egl_window_create".as_ptr()) };
        let resize = unsafe { dlsym(library, c"wl_egl_window_resize".as_ptr()) };
        let destroy = unsafe { dlsym(library, c"wl_egl_window_destroy".as_ptr()) };
        if create.is_null() || resize.is_null() || destroy.is_null() {
            unsafe { dlclose(library) };
            return Err("libwayland-egl.so.1 is missing required functions".to_owned());
        }
        let create: WaylandEglWindowCreate = unsafe { std::mem::transmute(create) };
        let resize: WaylandEglWindowResize = unsafe { std::mem::transmute(resize) };
        let destroy: WaylandEglWindowDestroy = unsafe { std::mem::transmute(destroy) };
        let window = unsafe { create(surface, size.0.max(1) as i32, size.1.max(1) as i32) };
        if window.is_null() {
            unsafe { dlclose(library) };
            return Err("wl_egl_window_create failed".to_owned());
        }
        Ok(Self {
            library,
            window,
            resize,
            destroy,
        })
    }
}

impl Drop for WaylandEgl {
    fn drop(&mut self) {
        unsafe {
            (self.destroy)(self.window);
            dlclose(self.library);
        }
    }
}

pub struct Context {
    egl: EglFns,
    display: EglDisplay,
    surface: EglSurface,
    context: EglContext,
    wayland: Option<WaylandEgl>,
    _gl_library: *mut c_void,
}

impl Context {
    pub fn new_headless() -> Result<Self, String> {
        let egl = EglFns::load()?;
        let get_platform_display = egl.get_platform_display.or_else(|| {
            let address = unsafe { egl.proc(c"eglGetPlatformDisplayEXT") };
            (!address.is_null()).then(|| unsafe { std::mem::transmute(address) })
        });
        let mut display = get_platform_display
            .map(|get_platform_display| unsafe {
                get_platform_display(EGL_PLATFORM_SURFACELESS_MESA, EGL_DEFAULT_DISPLAY, null())
            })
            .unwrap_or(null_mut());
        if display.is_null() {
            display = unsafe { (egl.get_display)(EGL_DEFAULT_DISPLAY) };
        }
        if display.is_null() {
            return Err(format!(
                "EGL could not open a headless display (error {:#x})",
                unsafe { (egl.get_error)() }
            ));
        }
        let mut egl_major = 0;
        let mut egl_minor = 0;
        if unsafe { (egl.initialize)(display, &mut egl_major, &mut egl_minor) } == EGL_FALSE {
            return Err(format!(
                "eglInitialize failed for the headless display (error {:#x})",
                unsafe { (egl.get_error)() }
            ));
        }
        #[cfg(target_os = "android")]
        let gl_api = EGL_OPENGL_ES_API;
        #[cfg(not(target_os = "android"))]
        let gl_api = EGL_OPENGL_API;
        if unsafe { (egl.bind_api)(gl_api) } == EGL_FALSE {
            unsafe { (egl.terminate)(display) };
            return Err(format!("eglBindAPI(OpenGL) failed (error {:#x})", unsafe {
                (egl.get_error)()
            }));
        }
        #[cfg(target_os = "android")]
        let renderable_type = EGL_OPENGL_ES3_BIT;
        #[cfg(not(target_os = "android"))]
        let renderable_type = EGL_OPENGL_BIT;
        let mut attributes = vec![
            EGL_SURFACE_TYPE,
            EGL_PBUFFER_BIT,
            EGL_RENDERABLE_TYPE,
            renderable_type,
            EGL_RED_SIZE,
            8,
            EGL_GREEN_SIZE,
            8,
            EGL_BLUE_SIZE,
            8,
            EGL_ALPHA_SIZE,
            8,
        ];
        #[cfg(not(target_os = "android"))]
        attributes.extend([EGL_DEPTH_SIZE, 24, EGL_STENCIL_SIZE, 8]);
        attributes.push(EGL_NONE);
        let mut config = null_mut();
        let mut config_count = 0;
        if unsafe {
            (egl.choose_config)(
                display,
                attributes.as_ptr(),
                &mut config,
                1,
                &mut config_count,
            )
        } == EGL_FALSE
            || config_count == 0
        {
            unsafe { (egl.terminate)(display) };
            return Err(format!(
                "EGL found no headless OpenGL pbuffer config (error {:#x})",
                unsafe { (egl.get_error)() }
            ));
        }
        let pbuffer_attributes = [EGL_WIDTH, 1, EGL_HEIGHT, 1, EGL_NONE];
        let surface =
            unsafe { (egl.create_pbuffer_surface)(display, config, pbuffer_attributes.as_ptr()) };
        if surface.is_null() {
            unsafe { (egl.terminate)(display) };
            return Err(format!(
                "eglCreatePbufferSurface failed (error {:#x})",
                unsafe { (egl.get_error)() }
            ));
        }
        #[cfg(target_os = "android")]
        let context_attributes = [EGL_CONTEXT_CLIENT_VERSION, 3, EGL_NONE];
        #[cfg(not(target_os = "android"))]
        let context_attributes = [
            EGL_CONTEXT_MAJOR_VERSION_KHR,
            3,
            EGL_CONTEXT_MINOR_VERSION_KHR,
            3,
            EGL_CONTEXT_OPENGL_PROFILE_MASK_KHR,
            EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT_KHR,
            EGL_NONE,
        ];
        let context = unsafe {
            (egl.create_context)(display, config, null_mut(), context_attributes.as_ptr())
        };
        if context.is_null() {
            unsafe {
                (egl.destroy_surface)(display, surface);
                (egl.terminate)(display);
            }
            return Err(format!(
                "EGL could not create a headless OpenGL 3.3 context (error {:#x})",
                unsafe { (egl.get_error)() }
            ));
        }
        if unsafe { (egl.make_current)(display, surface, surface, context) } == EGL_FALSE {
            unsafe {
                (egl.destroy_context)(display, context);
                (egl.destroy_surface)(display, surface);
                (egl.terminate)(display);
            }
            return Err(format!(
                "eglMakeCurrent failed for the headless context (error {:#x})",
                unsafe { (egl.get_error)() }
            ));
        }
        #[cfg(target_os = "android")]
        let gl_library_name = c"libGLESv3.so";
        #[cfg(not(target_os = "android"))]
        let gl_library_name = c"libGL.so.1";
        let gl_library = unsafe { dlopen(gl_library_name.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
        Ok(Self {
            egl,
            display,
            surface,
            context,
            wayland: None,
            _gl_library: gl_library,
        })
    }

    pub fn new(
        display_handle: RawDisplayHandle,
        window_handle: RawWindowHandle,
        size: (u32, u32),
    ) -> Result<Self, String> {
        let egl = EglFns::load()?;
        let (platform, native_display, native_window, visual_id, wayland_surface) =
            native_handles(display_handle, window_handle)?;
        let display = if cfg!(target_os = "android") {
            unsafe { (egl.get_display)(native_display) }
        } else if let Some(get_platform_display) = egl.get_platform_display.or_else(|| {
            let address = unsafe { egl.proc(c"eglGetPlatformDisplayEXT") };
            (!address.is_null()).then(|| unsafe { std::mem::transmute(address) })
        }) {
            unsafe { get_platform_display(platform, native_display, null()) }
        } else if matches!(
            display_handle,
            RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_)
        ) {
            unsafe { (egl.get_display)(native_display) }
        } else {
            null_mut()
        };
        if display.is_null() {
            return Err(format!(
                "EGL could not open the native display (error {:#x})",
                unsafe { (egl.get_error)() }
            ));
        }

        let mut egl_major = 0;
        let mut egl_minor = 0;
        if unsafe { (egl.initialize)(display, &mut egl_major, &mut egl_minor) } == EGL_FALSE {
            return Err(format!("eglInitialize failed (error {:#x})", unsafe {
                (egl.get_error)()
            }));
        }
        #[cfg(target_os = "android")]
        let gl_api = EGL_OPENGL_ES_API;
        #[cfg(not(target_os = "android"))]
        let gl_api = EGL_OPENGL_API;
        if unsafe { (egl.bind_api)(gl_api) } == EGL_FALSE {
            unsafe { (egl.terminate)(display) };
            return Err(format!("eglBindAPI(OpenGL) failed (error {:#x})", unsafe {
                (egl.get_error)()
            }));
        }

        #[cfg(target_os = "android")]
        let renderable_type = EGL_OPENGL_ES3_BIT;
        #[cfg(not(target_os = "android"))]
        let renderable_type = EGL_OPENGL_BIT;
        let mut config_attributes = vec![
            EGL_SURFACE_TYPE,
            EGL_WINDOW_BIT,
            EGL_RENDERABLE_TYPE,
            renderable_type,
            EGL_RED_SIZE,
            8,
            EGL_GREEN_SIZE,
            8,
            EGL_BLUE_SIZE,
            8,
            EGL_ALPHA_SIZE,
            8,
        ];
        #[cfg(not(target_os = "android"))]
        config_attributes.extend([EGL_DEPTH_SIZE, 24, EGL_STENCIL_SIZE, 8]);
        if let Some(visual_id) = visual_id {
            config_attributes.extend([EGL_NATIVE_VISUAL_ID, visual_id as i32]);
        }
        config_attributes.push(EGL_NONE);
        let mut config = null_mut();
        let mut config_count = 0;
        if unsafe {
            (egl.choose_config)(
                display,
                config_attributes.as_ptr(),
                &mut config,
                1,
                &mut config_count,
            )
        } == EGL_FALSE
            || config_count == 0
        {
            unsafe { (egl.terminate)(display) };
            return Err(format!(
                "eglChooseConfig found no desktop OpenGL window config (error {:#x})",
                unsafe { (egl.get_error)() }
            ));
        }

        let wayland = if let Some(surface) = wayland_surface {
            match WaylandEgl::new(surface, size) {
                Ok(wayland) => Some(wayland),
                Err(error) => {
                    unsafe { (egl.terminate)(display) };
                    return Err(error);
                }
            }
        } else {
            None
        };
        let native_window = wayland
            .as_ref()
            .map_or(native_window, |wayland| wayland.window);
        let create_window_surface = egl.create_platform_window_surface.or_else(|| {
            let address = unsafe { egl.proc(c"eglCreatePlatformWindowSurfaceEXT") };
            (!address.is_null()).then(|| unsafe { std::mem::transmute(address) })
        });
        let surface = if let Some(create_surface) = create_window_surface {
            unsafe { create_surface(display, config, native_window, null()) }
        } else {
            unsafe { (egl.create_window_surface)(display, config, native_window, null()) }
        };
        if surface.is_null() {
            unsafe { (egl.terminate)(display) };
            return Err(format!(
                "EGL could not create a window surface (error {:#x})",
                unsafe { (egl.get_error)() }
            ));
        }

        #[cfg(target_os = "android")]
        let context_attributes = [EGL_CONTEXT_CLIENT_VERSION, 3, EGL_NONE];
        #[cfg(not(target_os = "android"))]
        let context_attributes = [
            EGL_CONTEXT_MAJOR_VERSION_KHR,
            3,
            EGL_CONTEXT_MINOR_VERSION_KHR,
            3,
            EGL_CONTEXT_OPENGL_PROFILE_MASK_KHR,
            EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT_KHR,
            EGL_NONE,
        ];
        let context = unsafe {
            (egl.create_context)(display, config, null_mut(), context_attributes.as_ptr())
        };
        if context.is_null() {
            unsafe {
                (egl.destroy_surface)(display, surface);
                (egl.terminate)(display);
            }
            return Err(format!(
                "EGL could not create an OpenGL 3.3 Core context (error {:#x})",
                unsafe { (egl.get_error)() }
            ));
        }
        if unsafe { (egl.make_current)(display, surface, surface, context) } == EGL_FALSE {
            unsafe {
                (egl.destroy_context)(display, context);
                (egl.destroy_surface)(display, surface);
                (egl.terminate)(display);
            }
            return Err(format!("eglMakeCurrent failed (error {:#x})", unsafe {
                (egl.get_error)()
            }));
        }
        unsafe { (egl.swap_interval)(display, 1) };

        #[cfg(target_os = "android")]
        let gl_library_name = c"libGLESv3.so";
        #[cfg(not(target_os = "android"))]
        let gl_library_name = c"libGL.so.1";
        let gl_library = unsafe { dlopen(gl_library_name.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
        Ok(Self {
            egl,
            display,
            surface,
            context,
            wayland,
            _gl_library: gl_library,
        })
    }

    pub fn make_current(&self) -> Result<(), String> {
        if unsafe {
            (self.egl.make_current)(self.display, self.surface, self.surface, self.context)
        } == EGL_FALSE
        {
            return Err(format!("eglMakeCurrent failed (error {:#x})", unsafe {
                (self.egl.get_error)()
            }));
        }
        Ok(())
    }

    pub fn swap_buffers(&self) -> Result<(), String> {
        if unsafe { (self.egl.swap_buffers)(self.display, self.surface) } == EGL_FALSE {
            return Err(format!("eglSwapBuffers failed (error {:#x})", unsafe {
                (self.egl.get_error)()
            }));
        }
        Ok(())
    }

    pub fn resize(&self, size: (u32, u32)) -> Result<(), String> {
        if let Some(wayland) = &self.wayland {
            unsafe {
                (wayland.resize)(
                    wayland.window,
                    size.0.max(1) as i32,
                    size.1.max(1) as i32,
                    0,
                    0,
                )
            };
        }
        self.make_current()
    }

    pub fn proc_address(&self, name: &CStr) -> *const c_void {
        let proc = unsafe { (self.egl.get_proc_address)(name.as_ptr()) };
        if !proc.is_null() {
            return proc;
        }
        if !self._gl_library.is_null() {
            return unsafe { dlsym(self._gl_library, name.as_ptr()) };
        }
        null()
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        unsafe {
            (self.egl.make_current)(self.display, null_mut(), null_mut(), null_mut());
            (self.egl.destroy_context)(self.display, self.context);
            (self.egl.destroy_surface)(self.display, self.surface);
            (self.egl.terminate)(self.display);
            if !self._gl_library.is_null() {
                dlclose(self._gl_library);
            }
        }
    }
}

fn native_handles(
    display: RawDisplayHandle,
    window: RawWindowHandle,
) -> Result<
    (
        EglEnum,
        *mut c_void,
        *mut c_void,
        Option<u32>,
        Option<*mut c_void>,
    ),
    String,
> {
    match (display, window) {
        (RawDisplayHandle::Xlib(display), RawWindowHandle::Xlib(window)) => Ok((
            EGL_PLATFORM_X11_EXT,
            display
                .display
                .map_or(EGL_DEFAULT_DISPLAY, |display| display.as_ptr()),
            window.window as usize as *mut c_void,
            (window.visual_id != 0).then_some(window.visual_id as u32),
            None,
        )),
        (RawDisplayHandle::Xcb(display), RawWindowHandle::Xcb(window)) => Ok((
            EGL_PLATFORM_XCB_EXT,
            display
                .connection
                .map_or(EGL_DEFAULT_DISPLAY, |connection| connection.as_ptr()),
            window.window.get() as usize as *mut c_void,
            window.visual_id.map(|id| id.get()),
            None,
        )),
        (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(window)) => Ok((
            EGL_PLATFORM_WAYLAND_EXT,
            display.display.as_ptr(),
            null_mut(),
            None,
            Some(window.surface.as_ptr()),
        )),
        (RawDisplayHandle::Android(_), RawWindowHandle::AndroidNdk(window)) => Ok((
            0,
            EGL_DEFAULT_DISPLAY,
            window.a_native_window.as_ptr(),
            None,
            None,
        )),
        _ => Err("OpenGL expects matching platform display/window handles".to_owned()),
    }
}
