//! Android: the native libraries the video path uses, loaded at run time.
//!
//! EGL and OpenGL ES convert the decoder's picture to RGBA, and
//! `AHardwareBuffer` is the memory that picture lands in. They are opened with
//! `dlopen` rather than linked, so building needs no NDK sysroot and an app
//! that never plays video never loads them.
//!
//! Everything here is used from one thread, the video worker, which owns the
//! EGL context.

use std::ffi::{CStr, c_char, c_void};
use std::ptr::{NonNull, null, null_mut};

type EglDisplay = *mut c_void;
type EglContext = *mut c_void;
type EglSurface = *mut c_void;
type EglConfig = *mut c_void;
type EglImage = *mut c_void;
type EglInt = i32;

const EGL_NONE: EglInt = 0x3038;
const EGL_SURFACE_TYPE: EglInt = 0x3033;
const EGL_PBUFFER_BIT: EglInt = 0x0001;
const EGL_RENDERABLE_TYPE: EglInt = 0x3040;
const EGL_OPENGL_ES3_BIT: EglInt = 0x0040;
const EGL_RED_SIZE: EglInt = 0x3024;
const EGL_GREEN_SIZE: EglInt = 0x3023;
const EGL_BLUE_SIZE: EglInt = 0x3022;
const EGL_ALPHA_SIZE: EglInt = 0x3021;
const EGL_CONTEXT_CLIENT_VERSION: EglInt = 0x3098;
const EGL_WIDTH: EglInt = 0x3057;
const EGL_HEIGHT: EglInt = 0x3056;
const EGL_NATIVE_BUFFER_ANDROID: u32 = 0x3140;
const EGL_IMAGE_PRESERVED_KHR: EglInt = 0x30D2;
const EGL_TRUE: EglInt = 1;

pub(crate) const GL_TEXTURE_EXTERNAL_OES: u32 = 0x8D65;
const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
const GL_TEXTURE_WRAP_S: u32 = 0x2802;
const GL_TEXTURE_WRAP_T: u32 = 0x2803;
const GL_LINEAR: i32 = 0x2601;
const GL_CLAMP_TO_EDGE: i32 = 0x812F;
const GL_FRAMEBUFFER: u32 = 0x8D40;
const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
const GL_FRAMEBUFFER_COMPLETE: u32 = 0x8CD5;
const GL_VERTEX_SHADER: u32 = 0x8B31;
const GL_FRAGMENT_SHADER: u32 = 0x8B30;
const GL_COMPILE_STATUS: u32 = 0x8B81;
const GL_LINK_STATUS: u32 = 0x8B82;
const GL_INFO_LOG_LENGTH: u32 = 0x8B84;
const GL_TRIANGLE_STRIP: u32 = 0x0005;
const GL_FLOAT: u32 = 0x1406;
const GL_TEXTURE0: u32 = 0x84C0;

const AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM: u32 = 1;
const AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE: u64 = 1 << 8;
const AHARDWAREBUFFER_USAGE_GPU_COLOR_OUTPUT: u64 = 1 << 9;

/// `AHardwareBuffer_Desc`.
#[repr(C)]
struct HardwareBufferDesc {
    width: u32,
    height: u32,
    layers: u32,
    format: u32,
    usage: u64,
    stride: u32,
    rfu0: u32,
    rfu1: u64,
}

/// A loaded shared library.
struct Library(NonNull<c_void>);

impl Library {
    fn open(name: &CStr) -> Option<Self> {
        // SAFETY: a valid NUL-terminated name.
        NonNull::new(unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) }).map(Self)
    }

    /// The function `name`, as the pointer type `F`.
    ///
    /// # Safety
    ///
    /// `F` must be the `extern "C"` function pointer type of that symbol.
    unsafe fn function<F: Copy>(&self, name: &CStr) -> Option<F> {
        debug_assert_eq!(size_of::<F>(), size_of::<*mut c_void>());
        // SAFETY: a valid handle and name.
        let address = unsafe { libc::dlsym(self.0.as_ptr(), name.as_ptr()) };
        // SAFETY: the caller vouches for the type; sizes match (asserted).
        (!address.is_null()).then(|| unsafe { std::mem::transmute_copy::<*mut c_void, F>(&address) })
    }
}

// The library is never unloaded: the functions taken from it live as long as
// the process.

macro_rules! functions {
    ($library:expr, $($name:ident : $type:ty),+ $(,)?) => {{
        // SAFETY: each type is the C signature of the symbol it is loaded for.
        ($(unsafe { $library.function::<$type>(concat_cstr(stringify!($name))) }?),+)
    }};
}

fn concat_cstr(name: &'static str) -> &'static CStr {
    // Symbol names are written as identifiers, which have no NUL; leak one
    // terminated copy per name (a handful, once per process).
    let mut bytes = name.as_bytes().to_vec();
    bytes.push(0);
    let leaked: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    CStr::from_bytes_with_nul(leaked).expect("a symbol name has no NUL")
}

type FnGetDisplay = unsafe extern "C" fn(*mut c_void) -> EglDisplay;
type FnInitialize = unsafe extern "C" fn(EglDisplay, *mut EglInt, *mut EglInt) -> u32;
type FnChooseConfig = unsafe extern "C" fn(EglDisplay, *const EglInt, *mut EglConfig, EglInt, *mut EglInt) -> u32;
type FnCreateContext = unsafe extern "C" fn(EglDisplay, EglConfig, EglContext, *const EglInt) -> EglContext;
type FnCreatePbuffer = unsafe extern "C" fn(EglDisplay, EglConfig, *const EglInt) -> EglSurface;
type FnMakeCurrent = unsafe extern "C" fn(EglDisplay, EglSurface, EglSurface, EglContext) -> u32;
type FnDestroyContext = unsafe extern "C" fn(EglDisplay, EglContext) -> u32;
type FnDestroySurface = unsafe extern "C" fn(EglDisplay, EglSurface) -> u32;
type FnGetProcAddress = unsafe extern "C" fn(*const c_char) -> *mut c_void;
type FnGetNativeClientBuffer = unsafe extern "C" fn(*const c_void) -> *mut c_void;
type FnCreateImage = unsafe extern "C" fn(EglDisplay, EglContext, u32, *mut c_void, *const EglInt) -> EglImage;
type FnDestroyImage = unsafe extern "C" fn(EglDisplay, EglImage) -> u32;
type FnImageTarget = unsafe extern "C" fn(u32, EglImage);

/// EGL with a context current on this thread, and the image extension.
pub(crate) struct Egl {
    display: EglDisplay,
    context: EglContext,
    surface: EglSurface,
    destroy_context: FnDestroyContext,
    destroy_surface: FnDestroySurface,
    make_current: FnMakeCurrent,
    get_native_client_buffer: FnGetNativeClientBuffer,
    create_image: FnCreateImage,
    destroy_image: FnDestroyImage,
    image_target: FnImageTarget,
    _egl: Library,
}

impl Egl {
    /// A GLES 3 context on a 1 x 1 pbuffer, made current on this thread.
    pub(crate) fn new() -> Result<Self, String> {
        let egl = Library::open(c"libEGL.so").ok_or("libEGL is not available")?;
        let missing = || "libEGL lacks a function the video path needs".to_string();
        let (get_display, initialize, choose_config, create_context, create_pbuffer, make_current) = (|| {
            Some(functions!(
                egl,
                eglGetDisplay: FnGetDisplay,
                eglInitialize: FnInitialize,
                eglChooseConfig: FnChooseConfig,
                eglCreateContext: FnCreateContext,
                eglCreatePbufferSurface: FnCreatePbuffer,
                eglMakeCurrent: FnMakeCurrent,
            ))
        })()
        .ok_or_else(missing)?;
        let (destroy_context, destroy_surface, get_proc_address) = (|| {
            Some(functions!(
                egl,
                eglDestroyContext: FnDestroyContext,
                eglDestroySurface: FnDestroySurface,
                eglGetProcAddress: FnGetProcAddress,
            ))
        })()
        .ok_or_else(missing)?;

        // SAFETY: plain EGL calls with valid arguments, checked as they go.
        unsafe {
            let display = get_display(null_mut());
            if display.is_null() {
                return Err("EGL has no display".into());
            }
            let (mut major, mut minor) = (0, 0);
            if initialize(display, &mut major, &mut minor) == 0 {
                return Err("could not initialise EGL".into());
            }
            let config_attributes = [
                EGL_RENDERABLE_TYPE,
                EGL_OPENGL_ES3_BIT,
                EGL_SURFACE_TYPE,
                EGL_PBUFFER_BIT,
                EGL_RED_SIZE,
                8,
                EGL_GREEN_SIZE,
                8,
                EGL_BLUE_SIZE,
                8,
                EGL_ALPHA_SIZE,
                8,
                EGL_NONE,
            ];
            let mut config: EglConfig = null_mut();
            let mut count = 0;
            if choose_config(display, config_attributes.as_ptr(), &mut config, 1, &mut count) == 0 || count < 1 {
                return Err("no EGL config supports OpenGL ES 3".into());
            }
            let context_attributes = [EGL_CONTEXT_CLIENT_VERSION, 3, EGL_NONE];
            let context = create_context(display, config, null_mut(), context_attributes.as_ptr());
            if context.is_null() {
                return Err("could not create an OpenGL ES 3 context".into());
            }
            let surface_attributes = [EGL_WIDTH, 1, EGL_HEIGHT, 1, EGL_NONE];
            let surface = create_pbuffer(display, config, surface_attributes.as_ptr());
            if surface.is_null() {
                destroy_context(display, context);
                return Err("could not create an EGL surface".into());
            }
            if make_current(display, surface, surface, context) == 0 {
                destroy_surface(display, surface);
                destroy_context(display, context);
                return Err("could not make the EGL context current".into());
            }
            let proc = |name: &CStr| {
                let address = get_proc_address(name.as_ptr());
                (!address.is_null()).then_some(address)
            };
            let extensions = (|| {
                Some((
                    proc(c"eglGetNativeClientBufferANDROID")?,
                    proc(c"eglCreateImageKHR")?,
                    proc(c"eglDestroyImageKHR")?,
                    proc(c"glEGLImageTargetTexture2DOES")?,
                ))
            })();
            let Some((native_buffer, create_image, destroy_image, image_target)) = extensions else {
                make_current(display, null_mut(), null_mut(), null_mut());
                destroy_surface(display, surface);
                destroy_context(display, context);
                return Err("EGL cannot import an AHardwareBuffer on this device".into());
            };
            Ok(Self {
                display,
                context,
                surface,
                destroy_context,
                destroy_surface,
                make_current,
                get_native_client_buffer: std::mem::transmute::<*mut c_void, FnGetNativeClientBuffer>(native_buffer),
                create_image: std::mem::transmute::<*mut c_void, FnCreateImage>(create_image),
                destroy_image: std::mem::transmute::<*mut c_void, FnDestroyImage>(destroy_image),
                image_target: std::mem::transmute::<*mut c_void, FnImageTarget>(image_target),
                _egl: egl,
            })
        }
    }
}

impl Drop for Egl {
    fn drop(&mut self) {
        // SAFETY: the handles are the ones created in `new`, released once.
        unsafe {
            (self.make_current)(self.display, null_mut(), null_mut(), null_mut());
            (self.destroy_surface)(self.display, self.surface);
            (self.destroy_context)(self.display, self.context);
        }
    }
}

/// An `AHardwareBuffer` of RGBA pixels the GPU can render into and sample.
pub(crate) struct HardwareBuffer {
    raw: NonNull<c_void>,
    release: unsafe extern "C" fn(*mut c_void),
    _android: Library,
}

// SAFETY: an `AHardwareBuffer` is reference counted and usable from any thread.
unsafe impl Send for HardwareBuffer {}
unsafe impl Sync for HardwareBuffer {}

impl HardwareBuffer {
    pub(crate) fn new(width: u32, height: u32) -> Result<Self, String> {
        type Allocate = unsafe extern "C" fn(*const HardwareBufferDesc, *mut *mut c_void) -> i32;
        type Release = unsafe extern "C" fn(*mut c_void);
        let android = Library::open(c"libandroid.so").ok_or("libandroid is not available")?;
        let (allocate, release) = (|| Some(functions!(android, AHardwareBuffer_allocate: Allocate, AHardwareBuffer_release: Release)))()
            .ok_or("AHardwareBuffer needs Android 8")?;
        let desc = HardwareBufferDesc {
            width,
            height,
            layers: 1,
            format: AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM,
            usage: AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE | AHARDWAREBUFFER_USAGE_GPU_COLOR_OUTPUT,
            stride: 0,
            rfu0: 0,
            rfu1: 0,
        };
        let mut raw: *mut c_void = null_mut();
        // SAFETY: a valid description and out-pointer.
        let status = unsafe { allocate(&desc, &mut raw) };
        let raw = NonNull::new(raw).filter(|_| status == 0).ok_or("could not allocate a hardware buffer")?;
        Ok(Self {
            raw,
            release,
            _android: android,
        })
    }

    /// The `AHardwareBuffer*`.
    pub(crate) fn as_ptr(&self) -> *mut c_void {
        self.raw.as_ptr()
    }
}

impl Drop for HardwareBuffer {
    fn drop(&mut self) {
        // SAFETY: the buffer was allocated by `new` and is released once.
        unsafe { (self.release)(self.raw.as_ptr()) };
    }
}

type FnGenObjects = unsafe extern "C" fn(i32, *mut u32);
type FnDeleteObjects = unsafe extern "C" fn(i32, *const u32);
type FnBind = unsafe extern "C" fn(u32, u32);
type FnTexParameteri = unsafe extern "C" fn(u32, u32, i32);
type FnCreateShader = unsafe extern "C" fn(u32) -> u32;
type FnShaderSource = unsafe extern "C" fn(u32, i32, *const *const c_char, *const i32);
type FnCompile = unsafe extern "C" fn(u32);
type FnGetShaderiv = unsafe extern "C" fn(u32, u32, *mut i32);
type FnGetInfoLog = unsafe extern "C" fn(u32, i32, *mut i32, *mut c_char);
type FnCreateProgram = unsafe extern "C" fn() -> u32;
type FnAttach = unsafe extern "C" fn(u32, u32);
type FnLink = unsafe extern "C" fn(u32);
type FnUseProgram = unsafe extern "C" fn(u32);
type FnGetLocation = unsafe extern "C" fn(u32, *const c_char) -> i32;
type FnUniformMatrix4fv = unsafe extern "C" fn(i32, i32, u8, *const f32);
type FnUniform1i = unsafe extern "C" fn(i32, i32);
type FnEnableAttrib = unsafe extern "C" fn(u32);
type FnAttribPointer = unsafe extern "C" fn(u32, i32, u32, u8, i32, *const c_void);
type FnFramebufferTexture2D = unsafe extern "C" fn(u32, u32, u32, u32, i32);
type FnCheckFramebuffer = unsafe extern "C" fn(u32) -> u32;
type FnViewport = unsafe extern "C" fn(i32, i32, i32, i32);
type FnDrawArrays = unsafe extern "C" fn(u32, i32, i32);
type FnFinish = unsafe extern "C" fn();
type FnActiveTexture = unsafe extern "C" fn(u32);
type FnDeleteShader = unsafe extern "C" fn(u32);
type FnDeleteProgram = unsafe extern "C" fn(u32);

/// The OpenGL ES calls the conversion pass uses.
pub(crate) struct Gles {
    gen_textures: FnGenObjects,
    delete_textures: FnDeleteObjects,
    bind_texture: FnBind,
    tex_parameteri: FnTexParameteri,
    create_shader: FnCreateShader,
    shader_source: FnShaderSource,
    compile_shader: FnCompile,
    get_shaderiv: FnGetShaderiv,
    get_shader_info_log: FnGetInfoLog,
    create_program: FnCreateProgram,
    attach_shader: FnAttach,
    link_program: FnLink,
    get_programiv: FnGetShaderiv,
    get_program_info_log: FnGetInfoLog,
    use_program: FnUseProgram,
    get_uniform_location: FnGetLocation,
    get_attrib_location: FnGetLocation,
    uniform_matrix4fv: FnUniformMatrix4fv,
    uniform1i: FnUniform1i,
    enable_vertex_attrib_array: FnEnableAttrib,
    vertex_attrib_pointer: FnAttribPointer,
    gen_framebuffers: FnGenObjects,
    delete_framebuffers: FnDeleteObjects,
    bind_framebuffer: FnBind,
    framebuffer_texture2d: FnFramebufferTexture2D,
    check_framebuffer_status: FnCheckFramebuffer,
    viewport: FnViewport,
    draw_arrays: FnDrawArrays,
    finish: FnFinish,
    active_texture: FnActiveTexture,
    delete_shader: FnDeleteShader,
    delete_program: FnDeleteProgram,
    _gles: Library,
}

impl Gles {
    pub(crate) fn new() -> Result<Self, String> {
        let gles = Library::open(c"libGLESv3.so")
            .or_else(|| Library::open(c"libGLESv2.so"))
            .ok_or("libGLESv3 is not available")?;
        let load = || {
            Some((
                functions!(
                    gles,
                    glGenTextures: FnGenObjects,
                    glDeleteTextures: FnDeleteObjects,
                    glBindTexture: FnBind,
                    glTexParameteri: FnTexParameteri,
                    glCreateShader: FnCreateShader,
                    glShaderSource: FnShaderSource,
                    glCompileShader: FnCompile,
                    glGetShaderiv: FnGetShaderiv,
                    glGetShaderInfoLog: FnGetInfoLog,
                    glCreateProgram: FnCreateProgram,
                    glAttachShader: FnAttach,
                    glLinkProgram: FnLink,
                    glGetProgramiv: FnGetShaderiv,
                    glGetProgramInfoLog: FnGetInfoLog,
                    glUseProgram: FnUseProgram,
                    glGetUniformLocation: FnGetLocation,
                ),
                functions!(
                    gles,
                    glGetAttribLocation: FnGetLocation,
                    glUniformMatrix4fv: FnUniformMatrix4fv,
                    glUniform1i: FnUniform1i,
                    glEnableVertexAttribArray: FnEnableAttrib,
                    glVertexAttribPointer: FnAttribPointer,
                    glGenFramebuffers: FnGenObjects,
                    glDeleteFramebuffers: FnDeleteObjects,
                    glBindFramebuffer: FnBind,
                    glFramebufferTexture2D: FnFramebufferTexture2D,
                    glCheckFramebufferStatus: FnCheckFramebuffer,
                    glViewport: FnViewport,
                    glDrawArrays: FnDrawArrays,
                    glFinish: FnFinish,
                    glActiveTexture: FnActiveTexture,
                    glDeleteShader: FnDeleteShader,
                    glDeleteProgram: FnDeleteProgram,
                ),
            ))
        };
        let (first, second) = load().ok_or("OpenGL ES lacks a function the video path needs")?;
        let (gen_textures, delete_textures, bind_texture, tex_parameteri) = (first.0, first.1, first.2, first.3);
        Ok(Self {
            gen_textures,
            delete_textures,
            bind_texture,
            tex_parameteri,
            create_shader: first.4,
            shader_source: first.5,
            compile_shader: first.6,
            get_shaderiv: first.7,
            get_shader_info_log: first.8,
            create_program: first.9,
            attach_shader: first.10,
            link_program: first.11,
            get_programiv: first.12,
            get_program_info_log: first.13,
            use_program: first.14,
            get_uniform_location: first.15,
            get_attrib_location: second.0,
            uniform_matrix4fv: second.1,
            uniform1i: second.2,
            enable_vertex_attrib_array: second.3,
            vertex_attrib_pointer: second.4,
            gen_framebuffers: second.5,
            delete_framebuffers: second.6,
            bind_framebuffer: second.7,
            framebuffer_texture2d: second.8,
            check_framebuffer_status: second.9,
            viewport: second.10,
            draw_arrays: second.11,
            finish: second.12,
            active_texture: second.13,
            delete_shader: second.14,
            delete_program: second.15,
            _gles: gles,
        })
    }
}

/// Positions and texture coordinates of the full-screen strip. The picture's
/// top lands on the buffer's first row, which Vulkan reads as the top: GL's
/// bottom (y = -1) is memory row 0, so the strip puts the picture's top
/// (v = 0 before the transform) there.
const STRIP: [f32; 16] = [
    -1.0, -1.0, 0.0, 1.0, //
    1.0, -1.0, 1.0, 1.0, //
    -1.0, 1.0, 0.0, 0.0, //
    1.0, 1.0, 1.0, 0.0,
];

const VERTEX: &CStr = c"#version 300 es
in vec2 position;
in vec2 coordinate;
uniform mat4 transform;
out vec2 varying_coordinate;
void main() {
    gl_Position = vec4(position, 0.0, 1.0);
    varying_coordinate = (transform * vec4(coordinate, 0.0, 1.0)).xy;
}";

const FRAGMENT: &CStr = c"#version 300 es
#extension GL_OES_EGL_image_external_essl3 : require
precision mediump float;
uniform samplerExternalOES picture;
in vec2 varying_coordinate;
out vec4 color;
void main() {
    color = texture(picture, varying_coordinate);
}";

/// A buffer's GL side: the image wrapping it, a texture of that image and a
/// framebuffer drawing into it.
pub(crate) struct Target {
    image: EglImage,
    texture: u32,
    framebuffer: u32,
}

/// The conversion pass: the decoder's external texture drawn into an RGBA
/// target.
pub(crate) struct Converter {
    gles: Gles,
    egl: Egl,
    program: u32,
    transform: i32,
    picture: i32,
    position: u32,
    coordinate: u32,
    /// The decoder's output texture, a `GL_TEXTURE_EXTERNAL_OES`.
    external: u32,
}

impl Converter {
    pub(crate) fn new() -> Result<Self, String> {
        let egl = Egl::new()?;
        let gles = Gles::new()?;
        // SAFETY: a current context on this thread; each call is checked.
        unsafe {
            let program = link(&gles, VERTEX, FRAGMENT)?;
            (gles.use_program)(program);
            let location = |name: &CStr| (gles.get_uniform_location)(program, name.as_ptr());
            let transform = location(c"transform");
            let picture = location(c"picture");
            let position = (gles.get_attrib_location)(program, c"position".as_ptr());
            let coordinate = (gles.get_attrib_location)(program, c"coordinate".as_ptr());
            if transform < 0 || picture < 0 || position < 0 || coordinate < 0 {
                return Err("the video shader lost an input".into());
            }
            let mut external = 0;
            (gles.gen_textures)(1, &mut external);
            (gles.bind_texture)(GL_TEXTURE_EXTERNAL_OES, external);
            for (parameter, value) in [
                (GL_TEXTURE_MIN_FILTER, GL_LINEAR),
                (GL_TEXTURE_MAG_FILTER, GL_LINEAR),
                (GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE),
                (GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE),
            ] {
                (gles.tex_parameteri)(GL_TEXTURE_EXTERNAL_OES, parameter, value);
            }
            Ok(Self {
                gles,
                egl,
                program,
                transform,
                picture,
                position: position as u32,
                coordinate: coordinate as u32,
                external,
            })
        }
    }

    /// The GL texture name the decoder's `SurfaceTexture` attaches to.
    pub(crate) fn external_texture(&self) -> u32 {
        self.external
    }

    /// Wraps `buffer` as a render target.
    pub(crate) fn target(&self, buffer: &HardwareBuffer) -> Result<Target, String> {
        // SAFETY: a current context; `buffer` is a live hardware buffer.
        unsafe {
            let client = (self.egl.get_native_client_buffer)(buffer.as_ptr());
            if client.is_null() {
                return Err("could not wrap the hardware buffer for EGL".into());
            }
            let attributes = [EGL_IMAGE_PRESERVED_KHR, EGL_TRUE, EGL_NONE];
            let image = (self.egl.create_image)(
                self.egl.display,
                null_mut(),
                EGL_NATIVE_BUFFER_ANDROID,
                client,
                attributes.as_ptr(),
            );
            if image.is_null() {
                return Err("could not create an EGL image of the hardware buffer".into());
            }
            let gles = &self.gles;
            let (mut texture, mut framebuffer) = (0, 0);
            (gles.gen_textures)(1, &mut texture);
            (gles.bind_texture)(GL_TEXTURE_2D, texture);
            (self.egl.image_target)(GL_TEXTURE_2D, image);
            (gles.gen_framebuffers)(1, &mut framebuffer);
            (gles.bind_framebuffer)(GL_FRAMEBUFFER, framebuffer);
            (gles.framebuffer_texture2d)(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, texture, 0);
            let complete = (gles.check_framebuffer_status)(GL_FRAMEBUFFER) == GL_FRAMEBUFFER_COMPLETE;
            (gles.bind_framebuffer)(GL_FRAMEBUFFER, 0);
            let target = Target { image, texture, framebuffer };
            if complete {
                Ok(target)
            } else {
                self.destroy_target(target);
                Err("the hardware buffer cannot be rendered into".into())
            }
        }
    }

    /// Draws the decoder's current picture, through `transform` (the
    /// `SurfaceTexture`'s column-major matrix), into `target`, and waits for
    /// the GPU to finish so the buffer can be handed to another API.
    pub(crate) fn draw(&self, target: &Target, size: (u32, u32), transform: &[f32; 16]) {
        let gles = &self.gles;
        // SAFETY: a current context; the target belongs to it.
        unsafe {
            (gles.bind_framebuffer)(GL_FRAMEBUFFER, target.framebuffer);
            (gles.viewport)(0, 0, size.0 as i32, size.1 as i32);
            (gles.use_program)(self.program);
            (gles.active_texture)(GL_TEXTURE0);
            (gles.bind_texture)(GL_TEXTURE_EXTERNAL_OES, self.external);
            (gles.uniform1i)(self.picture, 0);
            (gles.uniform_matrix4fv)(self.transform, 1, 0, transform.as_ptr());
            let stride = (4 * size_of::<f32>()) as i32;
            (gles.enable_vertex_attrib_array)(self.position);
            (gles.vertex_attrib_pointer)(self.position, 2, GL_FLOAT, 0, stride, STRIP.as_ptr().cast());
            (gles.enable_vertex_attrib_array)(self.coordinate);
            (gles.vertex_attrib_pointer)(self.coordinate, 2, GL_FLOAT, 0, stride, STRIP[2..].as_ptr().cast());
            (gles.draw_arrays)(GL_TRIANGLE_STRIP, 0, 4);
            (gles.finish)();
            (gles.bind_framebuffer)(GL_FRAMEBUFFER, 0);
        }
    }

    pub(crate) fn destroy_target(&self, target: Target) {
        // SAFETY: a current context; the target's objects are released once.
        unsafe {
            (self.gles.delete_framebuffers)(1, &target.framebuffer);
            (self.gles.delete_textures)(1, &target.texture);
            (self.egl.destroy_image)(self.egl.display, target.image);
        }
    }
}

impl Drop for Converter {
    fn drop(&mut self) {
        // SAFETY: a current context; released once.
        unsafe {
            (self.gles.delete_textures)(1, &self.external);
            (self.gles.delete_program)(self.program);
        }
    }
}

/// Compiles and links a program.
///
/// # Safety
///
/// A GLES context must be current.
unsafe fn link(gles: &Gles, vertex: &CStr, fragment: &CStr) -> Result<u32, String> {
    // SAFETY: as documented; every object is checked and released on failure.
    unsafe {
        let compile = |kind: u32, source: &CStr| -> Result<u32, String> {
            let shader = (gles.create_shader)(kind);
            let sources = [source.as_ptr()];
            (gles.shader_source)(shader, 1, sources.as_ptr(), null());
            (gles.compile_shader)(shader);
            let mut status = 0;
            (gles.get_shaderiv)(shader, GL_COMPILE_STATUS, &mut status);
            if status == 0 {
                let log = info_log(gles.get_shaderiv, gles.get_shader_info_log, shader);
                (gles.delete_shader)(shader);
                return Err(format!("the video shader does not compile: {log}"));
            }
            Ok(shader)
        };
        let vertex = compile(GL_VERTEX_SHADER, vertex)?;
        let fragment = match compile(GL_FRAGMENT_SHADER, fragment) {
            Ok(fragment) => fragment,
            Err(why) => {
                (gles.delete_shader)(vertex);
                return Err(why);
            }
        };
        let program = (gles.create_program)();
        (gles.attach_shader)(program, vertex);
        (gles.attach_shader)(program, fragment);
        (gles.link_program)(program);
        (gles.delete_shader)(vertex);
        (gles.delete_shader)(fragment);
        let mut status = 0;
        (gles.get_programiv)(program, GL_LINK_STATUS, &mut status);
        if status == 0 {
            let log = info_log(gles.get_programiv, gles.get_program_info_log, program);
            (gles.delete_program)(program);
            return Err(format!("the video shader does not link: {log}"));
        }
        Ok(program)
    }
}

/// A shader's or program's info log.
///
/// # Safety
///
/// A GLES context must be current and `object` valid for both functions.
unsafe fn info_log(get: FnGetShaderiv, log: FnGetInfoLog, object: u32) -> String {
    // SAFETY: as documented.
    unsafe {
        let mut length = 0;
        get(object, GL_INFO_LOG_LENGTH, &mut length);
        let mut buffer = vec![0_u8; length.max(1) as usize];
        let mut written = 0;
        log(object, buffer.len() as i32, &mut written, buffer.as_mut_ptr().cast());
        buffer.truncate(written.max(0) as usize);
        String::from_utf8_lossy(&buffer).trim().to_string()
    }
}
