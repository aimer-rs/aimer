use std::ffi::{CStr, c_char, c_void};
use std::mem::size_of;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicUsize, Ordering};

use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle,
};

type Hwnd = *mut c_void;
type Hdc = *mut c_void;
type Hglrc = *mut c_void;
type Hmodule = *mut c_void;
type Hinstance = *mut c_void;
type WndProc = Option<unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize>;

#[repr(C)]
struct WndClassW {
    style: u32,
    wnd_proc: WndProc,
    class_extra: i32,
    window_extra: i32,
    instance: Hinstance,
    icon: *mut c_void,
    cursor: *mut c_void,
    background: *mut c_void,
    menu_name: *const u16,
    class_name: *const u16,
}

const PFD_DRAW_TO_WINDOW: u32 = 0x0000_0004;
const PFD_SUPPORT_OPENGL: u32 = 0x0000_0020;
const PFD_DOUBLEBUFFER: u32 = 0x0000_0001;
const PFD_TYPE_RGBA: u8 = 0;
const PFD_MAIN_PLANE: i8 = 0;

const WGL_CONTEXT_MAJOR_VERSION_ARB: i32 = 0x2091;
const WGL_CONTEXT_MINOR_VERSION_ARB: i32 = 0x2092;
const WGL_CONTEXT_PROFILE_MASK_ARB: i32 = 0x9126;
const WGL_CONTEXT_CORE_PROFILE_BIT_ARB: i32 = 0x0000_0001;

#[repr(C)]
struct PixelFormatDescriptor {
    size: u16,
    version: u16,
    flags: u32,
    pixel_type: u8,
    color_bits: u8,
    red_bits: u8,
    red_shift: u8,
    green_bits: u8,
    green_shift: u8,
    blue_bits: u8,
    blue_shift: u8,
    alpha_bits: u8,
    alpha_shift: u8,
    accum_bits: u8,
    accum_red_bits: u8,
    accum_green_bits: u8,
    accum_blue_bits: u8,
    accum_alpha_bits: u8,
    depth_bits: u8,
    stencil_bits: u8,
    aux_buffers: u8,
    layer_type: i8,
    reserved: u8,
    layer_mask: u32,
    visible_mask: u32,
    damage_mask: u32,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn GetDC(window: Hwnd) -> Hdc;
    fn ReleaseDC(window: Hwnd, dc: Hdc) -> i32;
    fn GetModuleHandleW(name: *const u16) -> Hinstance;
    fn RegisterClassW(class: *const WndClassW) -> u16;
    fn UnregisterClassW(class_name: *const u16, instance: Hinstance) -> i32;
    fn CreateWindowExW(
        extended_style: u32,
        class_name: *const u16,
        window_name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: Hwnd,
        menu: *mut c_void,
        instance: Hinstance,
        parameter: *mut c_void,
    ) -> Hwnd;
    fn DestroyWindow(window: Hwnd) -> i32;
    fn DefWindowProcW(window: Hwnd, message: u32, wparam: usize, lparam: isize) -> isize;
}

#[link(name = "gdi32")]
unsafe extern "system" {
    fn ChoosePixelFormat(dc: Hdc, descriptor: *const PixelFormatDescriptor) -> i32;
    fn SetPixelFormat(dc: Hdc, format: i32, descriptor: *const PixelFormatDescriptor) -> i32;
    fn SwapBuffers(dc: Hdc) -> i32;
}

#[link(name = "opengl32")]
unsafe extern "system" {
    fn wglCreateContext(dc: Hdc) -> Hglrc;
    fn wglDeleteContext(context: Hglrc) -> i32;
    fn wglMakeCurrent(dc: Hdc, context: Hglrc) -> i32;
    fn wglGetProcAddress(name: *const c_char) -> *const c_void;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryA(name: *const c_char) -> Hmodule;
    fn FreeLibrary(module: Hmodule) -> i32;
    fn GetProcAddress(module: Hmodule, name: *const c_char) -> *const c_void;
}

type CreateContextAttribs = unsafe extern "system" fn(Hdc, Hglrc, *const i32) -> Hglrc;

pub struct Context {
    window: Hwnd,
    dc: Hdc,
    gl_library: Hmodule,
    context: Hglrc,
    owned_class: Option<(Vec<u16>, Hinstance)>,
}

impl Context {
    pub fn new_headless() -> Result<Self, String> {
        static NEXT_CLASS: AtomicUsize = AtomicUsize::new(0);
        const CS_OWNDC: u32 = 0x0020;
        const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
        const WS_POPUP: u32 = 0x8000_0000;

        let instance = unsafe { GetModuleHandleW(null_mut()) };
        if instance.is_null() {
            return Err("GetModuleHandleW failed for the hidden OpenGL test window".to_owned());
        }
        let class_name: Vec<u16> = format!(
            "AimerOpenGLHeadless{}",
            NEXT_CLASS.fetch_add(1, Ordering::Relaxed)
        )
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
        let class = WndClassW {
            style: CS_OWNDC,
            wnd_proc: Some(DefWindowProcW),
            class_extra: 0,
            window_extra: 0,
            instance,
            icon: null_mut(),
            cursor: null_mut(),
            background: null_mut(),
            menu_name: null_mut(),
            class_name: class_name.as_ptr(),
        };
        if unsafe { RegisterClassW(&class) } == 0 {
            return Err("RegisterClassW failed for the hidden OpenGL test window".to_owned());
        }
        let window = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class_name.as_ptr(),
                class_name.as_ptr(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                null_mut(),
                null_mut(),
                instance,
                null_mut(),
            )
        };
        if window.is_null() {
            unsafe { UnregisterClassW(class_name.as_ptr(), instance) };
            return Err("CreateWindowExW failed for the hidden OpenGL test window".to_owned());
        }
        let display = RawDisplayHandle::Windows(WindowsDisplayHandle::new());
        let window_handle = RawWindowHandle::Win32(Win32WindowHandle::new(
            std::num::NonZeroIsize::new(window as isize).expect("hidden window handle is non-zero"),
        ));
        match Self::new(display, window_handle, (1, 1)) {
            Ok(mut context) => {
                context.owned_class = Some((class_name, instance));
                Ok(context)
            }
            Err(error) => {
                unsafe {
                    DestroyWindow(window);
                    UnregisterClassW(class_name.as_ptr(), instance);
                }
                Err(error)
            }
        }
    }

    pub fn new(
        display: RawDisplayHandle,
        window: RawWindowHandle,
        _size: (u32, u32),
    ) -> Result<Self, String> {
        if !matches!(display, RawDisplayHandle::Windows(_)) {
            return Err("OpenGL WGL requires a Win32 display handle".to_owned());
        }
        let RawWindowHandle::Win32(handle) = window else {
            return Err("OpenGL WGL requires a Win32 window handle".to_owned());
        };
        let hwnd = handle.hwnd.get() as Hwnd;
        // SAFETY: Winit owns the HWND; the caller retains its window until this
        // context and every GL resource have been dropped.
        let dc = unsafe { GetDC(hwnd) };
        if dc.is_null() {
            return Err("GetDC failed for the OpenGL window".to_owned());
        }

        let descriptor = PixelFormatDescriptor {
            size: size_of::<PixelFormatDescriptor>() as u16,
            version: 1,
            flags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
            pixel_type: PFD_TYPE_RGBA,
            color_bits: 32,
            red_bits: 8,
            red_shift: 0,
            green_bits: 8,
            green_shift: 0,
            blue_bits: 8,
            blue_shift: 0,
            alpha_bits: 8,
            alpha_shift: 0,
            accum_bits: 0,
            accum_red_bits: 0,
            accum_green_bits: 0,
            accum_blue_bits: 0,
            accum_alpha_bits: 0,
            depth_bits: 24,
            stencil_bits: 8,
            aux_buffers: 0,
            layer_type: PFD_MAIN_PLANE,
            reserved: 0,
            layer_mask: 0,
            visible_mask: 0,
            damage_mask: 0,
        };
        // SAFETY: the descriptor is fully initialized and dc belongs to hwnd.
        let format = unsafe { ChoosePixelFormat(dc, &descriptor) };
        if format == 0 || unsafe { SetPixelFormat(dc, format, &descriptor) } == 0 {
            unsafe { ReleaseDC(hwnd, dc) };
            return Err("choose or set the OpenGL window pixel format failed".to_owned());
        }

        let gl_library_name = c"opengl32.dll";
        // SAFETY: loads the OS OpenGL implementation, which is released in Drop.
        let gl_library = unsafe { LoadLibraryA(gl_library_name.as_ptr()) };
        if gl_library.is_null() {
            unsafe { ReleaseDC(hwnd, dc) };
            return Err("load opengl32.dll failed".to_owned());
        }

        // WGL needs a current legacy context before wglGetProcAddress can load
        // the 3.3 context-creation extension.
        let bootstrap = unsafe { wglCreateContext(dc) };
        if bootstrap.is_null() || unsafe { wglMakeCurrent(dc, bootstrap) } == 0 {
            if !bootstrap.is_null() {
                unsafe { wglDeleteContext(bootstrap) };
            }
            unsafe {
                FreeLibrary(gl_library);
                ReleaseDC(hwnd, dc);
            }
            return Err("create the temporary WGL context failed".to_owned());
        }
        let proc = unsafe { wglGetProcAddress(c"wglCreateContextAttribsARB".as_ptr()) };
        if !valid_wgl_proc(proc) {
            unsafe {
                wglMakeCurrent(null_mut(), null_mut());
                wglDeleteContext(bootstrap);
                FreeLibrary(gl_library);
                ReleaseDC(hwnd, dc);
            }
            return Err("the OpenGL driver does not expose WGL_ARB_create_context".to_owned());
        }
        // SAFETY: the current context returned the address for this WGL entry point.
        let create_context: CreateContextAttribs = unsafe { std::mem::transmute(proc) };
        let attributes = [
            WGL_CONTEXT_MAJOR_VERSION_ARB,
            3,
            WGL_CONTEXT_MINOR_VERSION_ARB,
            3,
            WGL_CONTEXT_PROFILE_MASK_ARB,
            WGL_CONTEXT_CORE_PROFILE_BIT_ARB,
            0,
        ];
        let context = unsafe { create_context(dc, null_mut(), attributes.as_ptr()) };
        unsafe {
            wglMakeCurrent(null_mut(), null_mut());
            wglDeleteContext(bootstrap);
        }
        if context.is_null() || unsafe { wglMakeCurrent(dc, context) } == 0 {
            if !context.is_null() {
                unsafe { wglDeleteContext(context) };
            }
            unsafe {
                FreeLibrary(gl_library);
                ReleaseDC(hwnd, dc);
            }
            return Err("create or activate an OpenGL 3.3 Core context failed".to_owned());
        }

        Ok(Self {
            window: hwnd,
            dc,
            gl_library,
            context,
            owned_class: None,
        })
    }

    pub fn make_current(&self) -> Result<(), String> {
        // SAFETY: dc and context remain live for self's lifetime.
        if unsafe { wglMakeCurrent(self.dc, self.context) } == 0 {
            return Err("wglMakeCurrent failed".to_owned());
        }
        Ok(())
    }

    pub fn swap_buffers(&self) -> Result<(), String> {
        // SAFETY: the context is current on this thread and dc is its drawable.
        if unsafe { SwapBuffers(self.dc) } == 0 {
            return Err("SwapBuffers failed".to_owned());
        }
        Ok(())
    }

    pub fn resize(&self, _size: (u32, u32)) -> Result<(), String> {
        self.make_current()
    }

    pub fn proc_address(&self, name: &CStr) -> *const c_void {
        // SAFETY: wglGetProcAddress requires a current context, which callers
        // establish before the function table is loaded.
        let proc = unsafe { wglGetProcAddress(name.as_ptr()) };
        if valid_wgl_proc(proc) {
            return proc;
        }
        // SAFETY: both the library handle and NUL-terminated name are valid.
        unsafe { GetProcAddress(self.gl_library, name.as_ptr()) }
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        unsafe {
            wglMakeCurrent(self.dc, self.context);
            wglMakeCurrent(null_mut(), null_mut());
            wglDeleteContext(self.context);
            ReleaseDC(self.window, self.dc);
            FreeLibrary(self.gl_library);
            if let Some((class_name, instance)) = &self.owned_class {
                DestroyWindow(self.window);
                UnregisterClassW(class_name.as_ptr(), *instance);
            }
        }
    }
}

fn valid_wgl_proc(proc: *const c_void) -> bool {
    let address = proc as usize;
    !proc.is_null() && address > 3 && proc != (-1isize as *const c_void)
}
