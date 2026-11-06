//! Windows: `IFileOpenDialog`.
//!
//! `Show` is a modal call that owns its own message loop until the user
//! finishes, so it runs on Venus's offload pool in its own COM apartment and the
//! UI thread keeps running. The owner window is found on the UI thread, where
//! `GetActiveWindow` is meaningful, and handed to the worker as a number; the
//! dialog is modal to it.
//!
//! A shown dialog cannot be closed from outside, so dropping the future leaves
//! it up until the user dismisses it; the result is then discarded.
//!
//! Windows shows one filter at a time, but a picker with several filters means
//! their union, so the list starts with an entry that combines them all.

use std::ffi::c_void;
use std::path::PathBuf;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FileOpenDialog,
    IFileOpenDialog, SIGDN_FILESYSPATH,
};
use windows::core::{HRESULT, HSTRING, PCWSTR};

use crate::error::{PickError, PickResult};
use crate::filter::FileFilter;
use crate::future::{PickFuture, ready};
use crate::picked::PickedFile;
use crate::request::FilePicker;

/// `HRESULT_FROM_WIN32(ERROR_CANCELLED)`: what `Show` returns when the user
/// dismisses the dialog.
const CANCELLED: HRESULT = HRESULT(0x800704C7_u32 as i32);

pub(crate) fn pick(request: FilePicker) -> PickFuture<Vec<PickedFile>> {
    // SAFETY: `GetActiveWindow` takes no arguments and reads the calling
    // thread's state; it is called on the UI thread, before the work moves.
    let owner = unsafe { GetActiveWindow() }.0 as usize;
    let Some(offloaded) = aimer_venus::spawn_blocking(move || show(owner, &request)) else {
        return ready(Err(PickError::Platform(
            "no Venus runtime is installed on this thread to run the dialog on".into(),
        )));
    };
    Box::pin(async move {
        let paths = offloaded.await?;
        Ok(paths.into_iter().map(PickedFile::from_path).collect())
    })
}

/// Leaves the COM apartment the thread entered, on every exit path.
struct Apartment;

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: paired with the successful `CoInitializeEx` that created this.
        unsafe { CoUninitialize() };
    }
}

fn platform(error: windows::core::Error) -> PickError {
    PickError::Platform(error.to_string())
}

/// Shows the dialog and returns the chosen paths; empty if it was dismissed.
fn show(owner: usize, request: &FilePicker) -> PickResult<Vec<PathBuf>> {
    // SAFETY: a plain call on this worker thread; `S_FALSE` (already
    // initialized) also needs the matching `CoUninitialize`, so any success is
    // guarded.
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let _apartment = initialized.is_ok().then_some(Apartment);

    // SAFETY: `FileOpenDialog` is the system's coclass for `IFileOpenDialog`.
    let dialog: IFileOpenDialog =
        unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER) }.map_err(platform)?;

    // SAFETY (all calls below): methods on the dialog this thread owns, with
    // arguments that outlive each call.
    unsafe {
        let mut options = dialog.GetOptions().map_err(platform)? | FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST | FOS_PATHMUSTEXIST;
        if request.multiple {
            options |= FOS_ALLOWMULTISELECT;
        }
        dialog.SetOptions(options).map_err(platform)?;

        if let Some(title) = &request.title {
            dialog.SetTitle(&HSTRING::from(title.as_str())).map_err(platform)?;
        }

        // The wide strings must outlive `SetFileTypes`.
        let entries = file_types(&request.filters);
        let specs: Vec<COMDLG_FILTERSPEC> = entries
            .iter()
            .map(|(name, spec)| COMDLG_FILTERSPEC {
                pszName: PCWSTR(name.as_ptr()),
                pszSpec: PCWSTR(spec.as_ptr()),
            })
            .collect();
        if !specs.is_empty() {
            dialog.SetFileTypes(&specs).map_err(platform)?;
        }

        let owner = (owner != 0).then_some(HWND(owner as *mut c_void));
        if let Err(error) = dialog.Show(owner) {
            return if error.code() == CANCELLED {
                Ok(Vec::new())
            } else {
                Err(platform(error))
            };
        }

        let items = dialog.GetResults().map_err(platform)?;
        let count = items.GetCount().map_err(platform)?;
        let mut paths = Vec::with_capacity(count as usize);
        for index in 0..count {
            let item = items.GetItemAt(index).map_err(platform)?;
            let name = item.GetDisplayName(SIGDN_FILESYSPATH).map_err(platform)?;
            let path = name.to_string();
            // The shell allocated the string with the COM allocator.
            CoTaskMemFree(Some(name.0 as *const c_void));
            if let Ok(path) = path {
                paths.push(PathBuf::from(path));
            }
        }
        Ok(paths)
    }
}

/// The drop-down entries: a display name and a `*.a;*.b` pattern each, as
/// NUL-terminated UTF-16 strings.
fn file_types(filters: &[FileFilter]) -> Vec<(HSTRING, HSTRING)> {
    let patterns: Vec<(&str, String)> = filters
        .iter()
        .filter_map(|filter| {
            let pattern = pattern(&filter.resolved_extensions());
            (!pattern.is_empty()).then_some((filter.name(), pattern))
        })
        .collect();

    let mut entries = Vec::with_capacity(patterns.len() + 1);
    if patterns.len() > 1 {
        let combined: Vec<&str> = patterns.iter().map(|(_, pattern)| pattern.as_str()).collect();
        entries.push((HSTRING::from("All supported files"), HSTRING::from(combined.join(";"))));
    }
    for (name, pattern) in patterns {
        entries.push((HSTRING::from(name), HSTRING::from(pattern)));
    }
    entries
}

/// `*.png;*.jpg` for a list of extensions.
fn pattern(extensions: &[String]) -> String {
    extensions
        .iter()
        .map(|extension| format!("*.{extension}"))
        .collect::<Vec<_>>()
        .join(";")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_list_every_extension() {
        assert_eq!(pattern(&["png".into(), "jpg".into()]), "*.png;*.jpg");
        assert_eq!(pattern(&[]), "");
    }
}
