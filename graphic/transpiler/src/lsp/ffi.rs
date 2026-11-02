//! C-ABI exports for the WebAssembly build. A host (the VS Code extension) copies the request
//! into module memory, calls [`aimer_handle`], then reads the response back.
//!
//! ```text
//! ptr = aimer_alloc(len); write the UTF-8 request at ptr
//! n   = aimer_handle(ptr, len)         // runs the request, frees nothing
//! out = aimer_result_ptr()             // n bytes of UTF-8 JSON
//! aimer_free(ptr, len)
//! ```

use std::cell::RefCell;

thread_local! {
    static RESULT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Reserve `len` bytes in module memory for the host to fill.
#[unsafe(no_mangle)]
pub extern "C" fn aimer_alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(len.max(1));
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// Release a buffer from [`aimer_alloc`].
///
/// # Safety
/// `ptr` and `len` must be exactly what [`aimer_alloc`] was given and returned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aimer_free(ptr: *mut u8, len: usize) {
    drop(unsafe { Vec::from_raw_parts(ptr, 0, len.max(1)) });
}

/// Handle the request at `ptr..ptr+len`. Returns the response length; read it at [`aimer_result_ptr`].
///
/// # Safety
/// `ptr..ptr+len` must be readable memory, e.g. from [`aimer_alloc`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn aimer_handle(ptr: *const u8, len: usize) -> usize {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let response = super::handle(&String::from_utf8_lossy(bytes));
    RESULT.with(|r| {
        let mut r = r.borrow_mut();
        *r = response.into_bytes();
        r.len()
    })
}

/// Address of the response of the last [`aimer_handle`] call.
#[unsafe(no_mangle)]
pub extern "C" fn aimer_result_ptr() -> *const u8 {
    RESULT.with(|r| r.borrow().as_ptr())
}
