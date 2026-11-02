use std::ffi::{CStr, c_char, c_void};

pub type GLenum = u32;
pub type GLboolean = u8;
pub type GLbitfield = u32;
pub type GLint = i32;
pub type GLsizei = i32;
pub type GLubyte = u8;
pub type GLuint = u32;
pub type GLfloat = f32;
pub type GLintptr = isize;
pub type GLsizeiptr = isize;
pub type GLchar = c_char;

pub const FALSE: GLboolean = 0;
pub const TRUE: GLboolean = 1;

pub const VERSION: GLenum = 0x1F02;
pub const MAJOR_VERSION: GLenum = 0x821B;
pub const MINOR_VERSION: GLenum = 0x821C;
pub const MAX_TEXTURE_SIZE: GLenum = 0x0D33;
pub const MAX_TEXTURE_IMAGE_UNITS: GLenum = 0x8872;
pub const MAX_UNIFORM_BUFFER_BINDINGS: GLenum = 0x8A2F;
pub const MAX_VERTEX_ATTRIBS: GLenum = 0x8869;
pub const UNIFORM_BUFFER_OFFSET_ALIGNMENT: GLenum = 0x8A34;

pub const ARRAY_BUFFER: GLenum = 0x8892;
pub const ELEMENT_ARRAY_BUFFER: GLenum = 0x8893;
pub const UNIFORM_BUFFER: GLenum = 0x8A11;
pub const COPY_READ_BUFFER: GLenum = 0x8F36;
pub const COPY_WRITE_BUFFER: GLenum = 0x8F37;
pub const PIXEL_PACK_BUFFER: GLenum = 0x88EB;
pub const PIXEL_UNPACK_BUFFER: GLenum = 0x88EC;
pub const STATIC_DRAW: GLenum = 0x88E4;
pub const DYNAMIC_DRAW: GLenum = 0x88E8;
pub const STREAM_DRAW: GLenum = 0x88E0;

pub const TEXTURE0: GLenum = 0x84C0;
pub const TEXTURE_1D: GLenum = 0x0DE0;
pub const TEXTURE_1D_ARRAY: GLenum = 0x8C18;
pub const TEXTURE_2D: GLenum = 0x0DE1;
pub const TEXTURE_2D_ARRAY: GLenum = 0x8C1A;
pub const TEXTURE_2D_MULTISAMPLE: GLenum = 0x9100;
pub const TEXTURE_3D: GLenum = 0x806F;
pub const TEXTURE_MIN_FILTER: GLenum = 0x2801;
pub const TEXTURE_MAG_FILTER: GLenum = 0x2800;
pub const TEXTURE_WRAP_S: GLenum = 0x2802;
pub const TEXTURE_WRAP_T: GLenum = 0x2803;
pub const TEXTURE_WRAP_R: GLenum = 0x8072;
pub const TEXTURE_MIN_LOD: GLenum = 0x813A;
pub const TEXTURE_MAX_LOD: GLenum = 0x813B;
pub const TEXTURE_BASE_LEVEL: GLenum = 0x813C;
pub const TEXTURE_MAX_LEVEL: GLenum = 0x813D;
pub const TEXTURE_COMPARE_MODE: GLenum = 0x884C;
pub const TEXTURE_COMPARE_FUNC: GLenum = 0x884D;
pub const COMPARE_REF_TO_TEXTURE: GLenum = 0x884E;
pub const NONE: GLenum = 0;

pub const NEAREST: GLenum = 0x2600;
pub const LINEAR: GLenum = 0x2601;
pub const NEAREST_MIPMAP_NEAREST: GLenum = 0x2700;
pub const LINEAR_MIPMAP_NEAREST: GLenum = 0x2701;
pub const NEAREST_MIPMAP_LINEAR: GLenum = 0x2702;
pub const LINEAR_MIPMAP_LINEAR: GLenum = 0x2703;
pub const CLAMP_TO_EDGE: GLenum = 0x812F;
pub const REPEAT: GLenum = 0x2901;
pub const MIRRORED_REPEAT: GLenum = 0x8370;

pub const RED: GLenum = 0x1903;
pub const RG: GLenum = 0x8227;
pub const RGBA: GLenum = 0x1908;
pub const BGRA: GLenum = 0x80E1;
pub const UNSIGNED_BYTE: GLenum = 0x1401;
pub const BYTE: GLenum = 0x1400;
pub const UNSIGNED_SHORT: GLenum = 0x1403;
pub const SHORT: GLenum = 0x1402;
pub const HALF_FLOAT: GLenum = 0x140B;
pub const UNSIGNED_INT: GLenum = 0x1405;
pub const INT: GLenum = 0x1404;
pub const FLOAT: GLenum = 0x1406;
pub const RGBA8: GLenum = 0x8058;
pub const SRGB8_ALPHA8: GLenum = 0x8C43;
pub const R8: GLenum = 0x8229;
pub const RG8: GLenum = 0x822B;
pub const RGBA16F: GLenum = 0x881A;
pub const DEPTH_COMPONENT: GLenum = 0x1902;
pub const DEPTH_COMPONENT16: GLenum = 0x81A5;
pub const DEPTH_COMPONENT24: GLenum = 0x81A6;
pub const DEPTH_COMPONENT32F: GLenum = 0x8CAC;
pub const DEPTH_STENCIL: GLenum = 0x84F9;
pub const DEPTH24_STENCIL8: GLenum = 0x88F0;
pub const UNSIGNED_INT_24_8: GLenum = 0x84FA;

pub const PACK_ALIGNMENT: GLenum = 0x0D05;
pub const UNPACK_ALIGNMENT: GLenum = 0x0CF5;
pub const UNPACK_ROW_LENGTH: GLenum = 0x0CF2;
pub const UNPACK_IMAGE_HEIGHT: GLenum = 0x806E;
pub const PACK_ROW_LENGTH: GLenum = 0x0D02;

pub const FRAMEBUFFER: GLenum = 0x8D40;
pub const DRAW_FRAMEBUFFER: GLenum = 0x8CA9;
pub const READ_FRAMEBUFFER: GLenum = 0x8CA8;
pub const COLOR_ATTACHMENT0: GLenum = 0x8CE0;
pub const DEPTH_ATTACHMENT: GLenum = 0x8D00;
pub const STENCIL_ATTACHMENT: GLenum = 0x8D20;
pub const DEPTH_STENCIL_ATTACHMENT: GLenum = 0x821A;
pub const FRAMEBUFFER_COMPLETE: GLenum = 0x8CD5;
pub const COLOR_BUFFER_BIT: GLbitfield = 0x0000_4000;
pub const DEPTH_BUFFER_BIT: GLbitfield = 0x0000_0100;
pub const STENCIL_BUFFER_BIT: GLbitfield = 0x0000_0400;
pub const COLOR: GLenum = 0x1800;
pub const DEPTH: GLenum = 0x1801;
pub const STENCIL: GLenum = 0x1802;

pub const VERTEX_SHADER: GLenum = 0x8B31;
pub const FRAGMENT_SHADER: GLenum = 0x8B30;
pub const COMPILE_STATUS: GLenum = 0x8B81;
pub const LINK_STATUS: GLenum = 0x8B82;
pub const INFO_LOG_LENGTH: GLenum = 0x8B84;
pub const INVALID_INDEX: GLuint = 0xFFFF_FFFF;

pub const TRIANGLES: GLenum = 0x0004;
pub const TRIANGLE_STRIP: GLenum = 0x0005;
pub const LINES: GLenum = 0x0001;
pub const LINE_STRIP: GLenum = 0x0003;
pub const POINTS: GLenum = 0x0000;

pub const ZERO: GLenum = 0;
pub const ONE: GLenum = 1;
pub const SRC_COLOR: GLenum = 0x0300;
pub const ONE_MINUS_SRC_COLOR: GLenum = 0x0301;
pub const SRC_ALPHA: GLenum = 0x0302;
pub const ONE_MINUS_SRC_ALPHA: GLenum = 0x0303;
pub const DST_ALPHA: GLenum = 0x0304;
pub const ONE_MINUS_DST_ALPHA: GLenum = 0x0305;
pub const DST_COLOR: GLenum = 0x0306;
pub const ONE_MINUS_DST_COLOR: GLenum = 0x0307;
pub const SRC_ALPHA_SATURATE: GLenum = 0x0308;
pub const CONSTANT_COLOR: GLenum = 0x8001;
pub const ONE_MINUS_CONSTANT_COLOR: GLenum = 0x8002;
pub const FUNC_ADD: GLenum = 0x8006;
pub const FUNC_SUBTRACT: GLenum = 0x800A;
pub const FUNC_REVERSE_SUBTRACT: GLenum = 0x800B;
pub const MIN: GLenum = 0x8007;
pub const MAX: GLenum = 0x8008;

pub const NEVER: GLenum = 0x0200;
pub const LESS: GLenum = 0x0201;
pub const EQUAL: GLenum = 0x0202;
pub const LEQUAL: GLenum = 0x0203;
pub const GREATER: GLenum = 0x0204;
pub const NOTEQUAL: GLenum = 0x0205;
pub const GEQUAL: GLenum = 0x0206;
pub const ALWAYS: GLenum = 0x0207;
pub const FRONT: GLenum = 0x0404;
pub const BACK: GLenum = 0x0405;
pub const FRONT_AND_BACK: GLenum = 0x0408;
pub const CCW: GLenum = 0x0901;
pub const CW: GLenum = 0x0900;
pub const FILL: GLenum = 0x1B02;
pub const LINE: GLenum = 0x1B01;
pub const POINT: GLenum = 0x1B00;
pub const KEEP: GLenum = 0x1E00;
pub const REPLACE: GLenum = 0x1E01;
pub const INCR: GLenum = 0x1E02;
pub const DECR: GLenum = 0x1E03;
pub const INVERT: GLenum = 0x150A;
pub const INCR_WRAP: GLenum = 0x8507;
pub const DECR_WRAP: GLenum = 0x8508;

pub const BLEND: GLenum = 0x0BE2;
pub const CULL_FACE: GLenum = 0x0B44;
pub const DEPTH_TEST: GLenum = 0x0B71;
pub const STENCIL_TEST: GLenum = 0x0B90;
pub const SCISSOR_TEST: GLenum = 0x0C11;
pub const POLYGON_OFFSET_FILL: GLenum = 0x8037;
pub const MULTISAMPLE: GLenum = 0x809D;
pub const SAMPLE_ALPHA_TO_COVERAGE: GLenum = 0x809E;
pub const NO_ERROR: GLenum = 0;

pub(super) type LoadFunction<'a> = dyn Fn(&CStr) -> *const c_void + 'a;

#[derive(Clone)]
pub struct GlFns {
    pub get_string: unsafe extern "system" fn(GLenum) -> *const GLubyte,
    pub get_integerv: unsafe extern "system" fn(GLenum, *mut GLint),
    pub get_error: unsafe extern "system" fn() -> GLenum,
    pub enable: unsafe extern "system" fn(GLenum),
    pub disable: unsafe extern "system" fn(GLenum),
    pub viewport: unsafe extern "system" fn(GLint, GLint, GLsizei, GLsizei),
    pub scissor: unsafe extern "system" fn(GLint, GLint, GLsizei, GLsizei),
    pub clear_color: unsafe extern "system" fn(GLfloat, GLfloat, GLfloat, GLfloat),
    pub clear: unsafe extern "system" fn(GLbitfield),
    pub clear_buffer_fv: unsafe extern "system" fn(GLenum, GLint, *const GLfloat),
    pub clear_buffer_iv: unsafe extern "system" fn(GLenum, GLint, *const GLint),
    pub color_mask_i: unsafe extern "system" fn(GLuint, GLboolean, GLboolean, GLboolean, GLboolean),
    pub depth_mask: unsafe extern "system" fn(GLboolean),
    pub depth_func: unsafe extern "system" fn(GLenum),
    pub stencil_func_separate: unsafe extern "system" fn(GLenum, GLenum, GLint, GLuint),
    pub stencil_op_separate: unsafe extern "system" fn(GLenum, GLenum, GLenum, GLenum),
    pub stencil_mask_separate: unsafe extern "system" fn(GLenum, GLuint),
    pub blend_func_separate: unsafe extern "system" fn(GLenum, GLenum, GLenum, GLenum),
    pub blend_equation_separate: unsafe extern "system" fn(GLenum, GLenum),
    pub blend_color: unsafe extern "system" fn(GLfloat, GLfloat, GLfloat, GLfloat),
    pub cull_face: unsafe extern "system" fn(GLenum),
    pub front_face: unsafe extern "system" fn(GLenum),
    pub polygon_mode: unsafe extern "system" fn(GLenum, GLenum),
    pub polygon_offset: unsafe extern "system" fn(GLfloat, GLfloat),
    pub gen_buffers: unsafe extern "system" fn(GLsizei, *mut GLuint),
    pub delete_buffers: unsafe extern "system" fn(GLsizei, *const GLuint),
    pub bind_buffer: unsafe extern "system" fn(GLenum, GLuint),
    pub buffer_data: unsafe extern "system" fn(GLenum, GLsizeiptr, *const c_void, GLenum),
    pub buffer_sub_data: unsafe extern "system" fn(GLenum, GLintptr, GLsizeiptr, *const c_void),
    pub get_buffer_sub_data: unsafe extern "system" fn(GLenum, GLintptr, GLsizeiptr, *mut c_void),
    pub bind_buffer_range: unsafe extern "system" fn(GLenum, GLuint, GLuint, GLintptr, GLsizeiptr),
    pub gen_textures: unsafe extern "system" fn(GLsizei, *mut GLuint),
    pub delete_textures: unsafe extern "system" fn(GLsizei, *const GLuint),
    pub bind_texture: unsafe extern "system" fn(GLenum, GLuint),
    pub active_texture: unsafe extern "system" fn(GLenum),
    pub tex_parameter_i: unsafe extern "system" fn(GLenum, GLenum, GLint),
    pub pixel_store_i: unsafe extern "system" fn(GLenum, GLint),
    pub tex_image_1d: unsafe extern "system" fn(
        GLenum,
        GLint,
        GLint,
        GLsizei,
        GLint,
        GLenum,
        GLenum,
        *const c_void,
    ),
    pub tex_image_2d: unsafe extern "system" fn(
        GLenum,
        GLint,
        GLint,
        GLsizei,
        GLsizei,
        GLint,
        GLenum,
        GLenum,
        *const c_void,
    ),
    pub tex_image_3d: unsafe extern "system" fn(
        GLenum,
        GLint,
        GLint,
        GLsizei,
        GLsizei,
        GLsizei,
        GLint,
        GLenum,
        GLenum,
        *const c_void,
    ),
    pub tex_image_2d_multisample:
        unsafe extern "system" fn(GLenum, GLsizei, GLenum, GLsizei, GLsizei, GLboolean),
    pub tex_sub_image_1d:
        unsafe extern "system" fn(GLenum, GLint, GLint, GLsizei, GLenum, GLenum, *const c_void),
    pub tex_sub_image_2d: unsafe extern "system" fn(
        GLenum,
        GLint,
        GLint,
        GLint,
        GLsizei,
        GLsizei,
        GLenum,
        GLenum,
        *const c_void,
    ),
    pub tex_sub_image_3d: unsafe extern "system" fn(
        GLenum,
        GLint,
        GLint,
        GLint,
        GLint,
        GLsizei,
        GLsizei,
        GLsizei,
        GLenum,
        GLenum,
        *const c_void,
    ),
    pub gen_samplers: unsafe extern "system" fn(GLsizei, *mut GLuint),
    pub delete_samplers: unsafe extern "system" fn(GLsizei, *const GLuint),
    pub bind_sampler: unsafe extern "system" fn(GLuint, GLuint),
    pub sampler_parameter_i: unsafe extern "system" fn(GLuint, GLenum, GLint),
    pub sampler_parameter_f: unsafe extern "system" fn(GLuint, GLenum, GLfloat),
    pub create_shader: unsafe extern "system" fn(GLenum) -> GLuint,
    pub shader_source:
        unsafe extern "system" fn(GLuint, GLsizei, *const *const GLchar, *const GLint),
    pub compile_shader: unsafe extern "system" fn(GLuint),
    pub get_shader_i: unsafe extern "system" fn(GLuint, GLenum, *mut GLint),
    pub get_shader_info_log: unsafe extern "system" fn(GLuint, GLsizei, *mut GLsizei, *mut GLchar),
    pub delete_shader: unsafe extern "system" fn(GLuint),
    pub create_program: unsafe extern "system" fn() -> GLuint,
    pub attach_shader: unsafe extern "system" fn(GLuint, GLuint),
    pub link_program: unsafe extern "system" fn(GLuint),
    pub get_program_i: unsafe extern "system" fn(GLuint, GLenum, *mut GLint),
    pub get_program_info_log: unsafe extern "system" fn(GLuint, GLsizei, *mut GLsizei, *mut GLchar),
    pub delete_program: unsafe extern "system" fn(GLuint),
    pub use_program: unsafe extern "system" fn(GLuint),
    pub get_uniform_location: unsafe extern "system" fn(GLuint, *const GLchar) -> GLint,
    pub uniform_1i: unsafe extern "system" fn(GLint, GLint),
    pub get_uniform_block_index: unsafe extern "system" fn(GLuint, *const GLchar) -> GLuint,
    pub uniform_block_binding: unsafe extern "system" fn(GLuint, GLuint, GLuint),
    pub gen_vertex_arrays: unsafe extern "system" fn(GLsizei, *mut GLuint),
    pub delete_vertex_arrays: unsafe extern "system" fn(GLsizei, *const GLuint),
    pub bind_vertex_array: unsafe extern "system" fn(GLuint),
    pub enable_vertex_attrib_array: unsafe extern "system" fn(GLuint),
    pub vertex_attrib_pointer:
        unsafe extern "system" fn(GLuint, GLint, GLenum, GLboolean, GLsizei, *const c_void),
    pub vertex_attrib_i_pointer:
        unsafe extern "system" fn(GLuint, GLint, GLenum, GLsizei, *const c_void),
    pub vertex_attrib_divisor: unsafe extern "system" fn(GLuint, GLuint),
    pub draw_arrays_instanced: unsafe extern "system" fn(GLenum, GLint, GLsizei, GLsizei),
    pub draw_elements_instanced_base_vertex:
        unsafe extern "system" fn(GLenum, GLsizei, GLenum, *const c_void, GLsizei, GLint),
    pub gen_framebuffers: unsafe extern "system" fn(GLsizei, *mut GLuint),
    pub delete_framebuffers: unsafe extern "system" fn(GLsizei, *const GLuint),
    pub bind_framebuffer: unsafe extern "system" fn(GLenum, GLuint),
    pub framebuffer_texture: unsafe extern "system" fn(GLenum, GLenum, GLuint, GLint),
    pub framebuffer_texture_layer: unsafe extern "system" fn(GLenum, GLenum, GLuint, GLint, GLint),
    pub check_framebuffer_status: unsafe extern "system" fn(GLenum) -> GLenum,
    pub draw_buffers: unsafe extern "system" fn(GLsizei, *const GLenum),
    pub read_buffer: unsafe extern "system" fn(GLenum),
    pub blit_framebuffer: unsafe extern "system" fn(
        GLint,
        GLint,
        GLint,
        GLint,
        GLint,
        GLint,
        GLint,
        GLint,
        GLbitfield,
        GLenum,
    ),
    pub read_pixels:
        unsafe extern "system" fn(GLint, GLint, GLsizei, GLsizei, GLenum, GLenum, *mut c_void),
    pub flush: unsafe extern "system" fn(),
}

impl GlFns {
    pub(super) unsafe fn load(resolver: &LoadFunction<'_>) -> Result<Self, String> {
        macro_rules! gl {
            ($name:literal, $signature:ty) => {{
                let name =
                    unsafe { CStr::from_bytes_with_nul_unchecked(concat!($name, "\0").as_bytes()) };
                let address = resolver(name);
                if address.is_null() {
                    return Err(format!("OpenGL function {} is unavailable", $name));
                }
                unsafe { std::mem::transmute::<*const c_void, $signature>(address) }
            }};
        }
        Ok(Self {
            get_string: gl!(
                "glGetString",
                unsafe extern "system" fn(GLenum) -> *const GLubyte
            ),
            get_integerv: gl!(
                "glGetIntegerv",
                unsafe extern "system" fn(GLenum, *mut GLint)
            ),
            get_error: gl!("glGetError", unsafe extern "system" fn() -> GLenum),
            enable: gl!("glEnable", unsafe extern "system" fn(GLenum)),
            disable: gl!("glDisable", unsafe extern "system" fn(GLenum)),
            viewport: gl!(
                "glViewport",
                unsafe extern "system" fn(GLint, GLint, GLsizei, GLsizei)
            ),
            scissor: gl!(
                "glScissor",
                unsafe extern "system" fn(GLint, GLint, GLsizei, GLsizei)
            ),
            clear_color: gl!(
                "glClearColor",
                unsafe extern "system" fn(GLfloat, GLfloat, GLfloat, GLfloat)
            ),
            clear: gl!("glClear", unsafe extern "system" fn(GLbitfield)),
            clear_buffer_fv: gl!(
                "glClearBufferfv",
                unsafe extern "system" fn(GLenum, GLint, *const GLfloat)
            ),
            clear_buffer_iv: gl!(
                "glClearBufferiv",
                unsafe extern "system" fn(GLenum, GLint, *const GLint)
            ),
            color_mask_i: gl!(
                "glColorMaski",
                unsafe extern "system" fn(GLuint, GLboolean, GLboolean, GLboolean, GLboolean)
            ),
            depth_mask: gl!("glDepthMask", unsafe extern "system" fn(GLboolean)),
            depth_func: gl!("glDepthFunc", unsafe extern "system" fn(GLenum)),
            stencil_func_separate: gl!(
                "glStencilFuncSeparate",
                unsafe extern "system" fn(GLenum, GLenum, GLint, GLuint)
            ),
            stencil_op_separate: gl!(
                "glStencilOpSeparate",
                unsafe extern "system" fn(GLenum, GLenum, GLenum, GLenum)
            ),
            stencil_mask_separate: gl!(
                "glStencilMaskSeparate",
                unsafe extern "system" fn(GLenum, GLuint)
            ),
            blend_func_separate: gl!(
                "glBlendFuncSeparate",
                unsafe extern "system" fn(GLenum, GLenum, GLenum, GLenum)
            ),
            blend_equation_separate: gl!(
                "glBlendEquationSeparate",
                unsafe extern "system" fn(GLenum, GLenum)
            ),
            blend_color: gl!(
                "glBlendColor",
                unsafe extern "system" fn(GLfloat, GLfloat, GLfloat, GLfloat)
            ),
            cull_face: gl!("glCullFace", unsafe extern "system" fn(GLenum)),
            front_face: gl!("glFrontFace", unsafe extern "system" fn(GLenum)),
            polygon_mode: gl!("glPolygonMode", unsafe extern "system" fn(GLenum, GLenum)),
            polygon_offset: gl!(
                "glPolygonOffset",
                unsafe extern "system" fn(GLfloat, GLfloat)
            ),
            gen_buffers: gl!(
                "glGenBuffers",
                unsafe extern "system" fn(GLsizei, *mut GLuint)
            ),
            delete_buffers: gl!(
                "glDeleteBuffers",
                unsafe extern "system" fn(GLsizei, *const GLuint)
            ),
            bind_buffer: gl!("glBindBuffer", unsafe extern "system" fn(GLenum, GLuint)),
            buffer_data: gl!(
                "glBufferData",
                unsafe extern "system" fn(GLenum, GLsizeiptr, *const c_void, GLenum)
            ),
            buffer_sub_data: gl!(
                "glBufferSubData",
                unsafe extern "system" fn(GLenum, GLintptr, GLsizeiptr, *const c_void)
            ),
            get_buffer_sub_data: gl!(
                "glGetBufferSubData",
                unsafe extern "system" fn(GLenum, GLintptr, GLsizeiptr, *mut c_void)
            ),
            bind_buffer_range: gl!(
                "glBindBufferRange",
                unsafe extern "system" fn(GLenum, GLuint, GLuint, GLintptr, GLsizeiptr)
            ),
            gen_textures: gl!(
                "glGenTextures",
                unsafe extern "system" fn(GLsizei, *mut GLuint)
            ),
            delete_textures: gl!(
                "glDeleteTextures",
                unsafe extern "system" fn(GLsizei, *const GLuint)
            ),
            bind_texture: gl!("glBindTexture", unsafe extern "system" fn(GLenum, GLuint)),
            active_texture: gl!("glActiveTexture", unsafe extern "system" fn(GLenum)),
            tex_parameter_i: gl!(
                "glTexParameteri",
                unsafe extern "system" fn(GLenum, GLenum, GLint)
            ),
            pixel_store_i: gl!("glPixelStorei", unsafe extern "system" fn(GLenum, GLint)),
            tex_image_1d: gl!(
                "glTexImage1D",
                unsafe extern "system" fn(
                    GLenum,
                    GLint,
                    GLint,
                    GLsizei,
                    GLint,
                    GLenum,
                    GLenum,
                    *const c_void,
                )
            ),
            tex_image_2d: gl!(
                "glTexImage2D",
                unsafe extern "system" fn(
                    GLenum,
                    GLint,
                    GLint,
                    GLsizei,
                    GLsizei,
                    GLint,
                    GLenum,
                    GLenum,
                    *const c_void,
                )
            ),
            tex_image_3d: gl!(
                "glTexImage3D",
                unsafe extern "system" fn(
                    GLenum,
                    GLint,
                    GLint,
                    GLsizei,
                    GLsizei,
                    GLsizei,
                    GLint,
                    GLenum,
                    GLenum,
                    *const c_void,
                )
            ),
            tex_image_2d_multisample: gl!(
                "glTexImage2DMultisample",
                unsafe extern "system" fn(GLenum, GLsizei, GLenum, GLsizei, GLsizei, GLboolean)
            ),
            tex_sub_image_1d: gl!(
                "glTexSubImage1D",
                unsafe extern "system" fn(
                    GLenum,
                    GLint,
                    GLint,
                    GLsizei,
                    GLenum,
                    GLenum,
                    *const c_void,
                )
            ),
            tex_sub_image_2d: gl!(
                "glTexSubImage2D",
                unsafe extern "system" fn(
                    GLenum,
                    GLint,
                    GLint,
                    GLint,
                    GLsizei,
                    GLsizei,
                    GLenum,
                    GLenum,
                    *const c_void,
                )
            ),
            tex_sub_image_3d: gl!(
                "glTexSubImage3D",
                unsafe extern "system" fn(
                    GLenum,
                    GLint,
                    GLint,
                    GLint,
                    GLint,
                    GLsizei,
                    GLsizei,
                    GLsizei,
                    GLenum,
                    GLenum,
                    *const c_void,
                )
            ),
            gen_samplers: gl!(
                "glGenSamplers",
                unsafe extern "system" fn(GLsizei, *mut GLuint)
            ),
            delete_samplers: gl!(
                "glDeleteSamplers",
                unsafe extern "system" fn(GLsizei, *const GLuint)
            ),
            bind_sampler: gl!("glBindSampler", unsafe extern "system" fn(GLuint, GLuint)),
            sampler_parameter_i: gl!(
                "glSamplerParameteri",
                unsafe extern "system" fn(GLuint, GLenum, GLint)
            ),
            sampler_parameter_f: gl!(
                "glSamplerParameterf",
                unsafe extern "system" fn(GLuint, GLenum, GLfloat)
            ),
            create_shader: gl!(
                "glCreateShader",
                unsafe extern "system" fn(GLenum) -> GLuint
            ),
            shader_source: gl!(
                "glShaderSource",
                unsafe extern "system" fn(GLuint, GLsizei, *const *const GLchar, *const GLint)
            ),
            compile_shader: gl!("glCompileShader", unsafe extern "system" fn(GLuint)),
            get_shader_i: gl!(
                "glGetShaderiv",
                unsafe extern "system" fn(GLuint, GLenum, *mut GLint)
            ),
            get_shader_info_log: gl!(
                "glGetShaderInfoLog",
                unsafe extern "system" fn(GLuint, GLsizei, *mut GLsizei, *mut GLchar)
            ),
            delete_shader: gl!("glDeleteShader", unsafe extern "system" fn(GLuint)),
            create_program: gl!("glCreateProgram", unsafe extern "system" fn() -> GLuint),
            attach_shader: gl!("glAttachShader", unsafe extern "system" fn(GLuint, GLuint)),
            link_program: gl!("glLinkProgram", unsafe extern "system" fn(GLuint)),
            get_program_i: gl!(
                "glGetProgramiv",
                unsafe extern "system" fn(GLuint, GLenum, *mut GLint)
            ),
            get_program_info_log: gl!(
                "glGetProgramInfoLog",
                unsafe extern "system" fn(GLuint, GLsizei, *mut GLsizei, *mut GLchar)
            ),
            delete_program: gl!("glDeleteProgram", unsafe extern "system" fn(GLuint)),
            use_program: gl!("glUseProgram", unsafe extern "system" fn(GLuint)),
            get_uniform_location: gl!(
                "glGetUniformLocation",
                unsafe extern "system" fn(GLuint, *const GLchar) -> GLint
            ),
            uniform_1i: gl!("glUniform1i", unsafe extern "system" fn(GLint, GLint)),
            get_uniform_block_index: gl!(
                "glGetUniformBlockIndex",
                unsafe extern "system" fn(GLuint, *const GLchar) -> GLuint
            ),
            uniform_block_binding: gl!(
                "glUniformBlockBinding",
                unsafe extern "system" fn(GLuint, GLuint, GLuint)
            ),
            gen_vertex_arrays: gl!(
                "glGenVertexArrays",
                unsafe extern "system" fn(GLsizei, *mut GLuint)
            ),
            delete_vertex_arrays: gl!(
                "glDeleteVertexArrays",
                unsafe extern "system" fn(GLsizei, *const GLuint)
            ),
            bind_vertex_array: gl!("glBindVertexArray", unsafe extern "system" fn(GLuint)),
            enable_vertex_attrib_array: gl!(
                "glEnableVertexAttribArray",
                unsafe extern "system" fn(GLuint)
            ),
            vertex_attrib_pointer: gl!(
                "glVertexAttribPointer",
                unsafe extern "system" fn(GLuint, GLint, GLenum, GLboolean, GLsizei, *const c_void)
            ),
            vertex_attrib_i_pointer: gl!(
                "glVertexAttribIPointer",
                unsafe extern "system" fn(GLuint, GLint, GLenum, GLsizei, *const c_void)
            ),
            vertex_attrib_divisor: gl!(
                "glVertexAttribDivisor",
                unsafe extern "system" fn(GLuint, GLuint)
            ),
            draw_arrays_instanced: gl!(
                "glDrawArraysInstanced",
                unsafe extern "system" fn(GLenum, GLint, GLsizei, GLsizei)
            ),
            draw_elements_instanced_base_vertex: gl!(
                "glDrawElementsInstancedBaseVertex",
                unsafe extern "system" fn(GLenum, GLsizei, GLenum, *const c_void, GLsizei, GLint)
            ),
            gen_framebuffers: gl!(
                "glGenFramebuffers",
                unsafe extern "system" fn(GLsizei, *mut GLuint)
            ),
            delete_framebuffers: gl!(
                "glDeleteFramebuffers",
                unsafe extern "system" fn(GLsizei, *const GLuint)
            ),
            bind_framebuffer: gl!(
                "glBindFramebuffer",
                unsafe extern "system" fn(GLenum, GLuint)
            ),
            framebuffer_texture: gl!(
                "glFramebufferTexture",
                unsafe extern "system" fn(GLenum, GLenum, GLuint, GLint)
            ),
            framebuffer_texture_layer: gl!(
                "glFramebufferTextureLayer",
                unsafe extern "system" fn(GLenum, GLenum, GLuint, GLint, GLint)
            ),
            check_framebuffer_status: gl!(
                "glCheckFramebufferStatus",
                unsafe extern "system" fn(GLenum) -> GLenum
            ),
            draw_buffers: gl!(
                "glDrawBuffers",
                unsafe extern "system" fn(GLsizei, *const GLenum)
            ),
            read_buffer: gl!("glReadBuffer", unsafe extern "system" fn(GLenum)),
            blit_framebuffer: gl!(
                "glBlitFramebuffer",
                unsafe extern "system" fn(
                    GLint,
                    GLint,
                    GLint,
                    GLint,
                    GLint,
                    GLint,
                    GLint,
                    GLint,
                    GLbitfield,
                    GLenum,
                )
            ),
            read_pixels: gl!(
                "glReadPixels",
                unsafe extern "system" fn(
                    GLint,
                    GLint,
                    GLsizei,
                    GLsizei,
                    GLenum,
                    GLenum,
                    *mut c_void,
                )
            ),
            flush: gl!("glFlush", unsafe extern "system" fn()),
        })
    }
}
