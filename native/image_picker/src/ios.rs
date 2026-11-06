//! iOS: `PHPickerViewController`.
//!
//! The photo picker runs out of process: the app never sees the library, only the
//! items the user taps, and needs no photo-library permission. Each chosen item
//! is an `NSItemProvider`; its file representation is only valid inside the
//! completion handler, which runs on a background queue, so the handler copies
//! the file into the app's temporary directory and the picked file is that copy.
//!
//! `objc2-photos-ui` binds the picker's configuration, filter and result types
//! everywhere but declares the view controller and its delegate for macOS only.
//! Both are therefore declared here, under their real Objective-C names, the way
//! `aimer_passkey` redeclares a protocol the bindings leave out for a platform.
//!
//! The delegate is held weakly by the picker and the picker does not dismiss
//! itself, so the future owns the delegate and the delegate dismisses the picker.
//! Dropping the future before the picker closes dismisses it.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use aimer_file_picker::{PickError, PickFuture, PickResult, PickedFile};
use aimer_venus::{Completer, completion};
use block2::RcBlock;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::ProtocolObject;
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, extern_class, extern_methods,
    extern_protocol, msg_send,
};
use objc2_foundation::{NSArray, NSError, NSInteger, NSObject, NSObjectProtocol, NSString, NSURL};
use objc2_photos_ui::{PHPickerConfiguration, PHPickerFilter, PHPickerResult};
use objc2_ui_kit::{
    UIApplication, UIResponder, UISceneActivationState, UIViewController, UIWindowScene,
};

use crate::picker::ImagePicker;

extern_class!(
    /// `PHPickerViewController` on iOS.
    // SAFETY: a `UIViewController` subclass with no extra subclassing requirements.
    #[unsafe(super(UIViewController, UIResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "PHPickerViewController"]
    struct PhotoPicker;
);

impl PhotoPicker {
    extern_methods!(
        #[unsafe(method(initWithConfiguration:))]
        #[unsafe(method_family = init)]
        fn init_with_configuration(this: Allocated<Self>, configuration: &PHPickerConfiguration) -> Retained<Self>;

        #[unsafe(method(setDelegate:))]
        #[unsafe(method_family = none)]
        fn set_delegate(&self, delegate: Option<&ProtocolObject<dyn PhotoPickerDelegate>>);
    );
}

extern_protocol!(
    /// `PHPickerViewControllerDelegate` on iOS.
    // SAFETY (for implementors): mirrors the framework's protocol; the method is
    // called on the main thread with the picker and its results.
    #[allow(clippy::missing_safety_doc)]
    #[name = "PHPickerViewControllerDelegate"]
    unsafe trait PhotoPickerDelegate: NSObjectProtocol + MainThreadOnly {
        #[unsafe(method(picker:didFinishPicking:))]
        #[unsafe(method_family = none)]
        unsafe fn picker_did_finish_picking(&self, picker: &PhotoPicker, results: &NSArray<PHPickerResult>);
    }
);

pub(crate) fn pick(request: &ImagePicker) -> PickFuture<Vec<PickedFile>> {
    let fail = |error| -> PickFuture<Vec<PickedFile>> { Box::pin(std::future::ready(Err(error))) };
    let Some(mtm) = MainThreadMarker::new() else {
        return fail(PickError::Platform(
            "the photo picker can only be shown from the main thread".into(),
        ));
    };
    let Some(presenter) = top_view_controller(mtm) else {
        return fail(PickError::NoWindow);
    };

    // SAFETY: plain configuration calls; a limit of 0 means "no limit" to the
    // picker, and the filter is a retained system object.
    let configuration = unsafe {
        let configuration = PHPickerConfiguration::new();
        configuration.setSelectionLimit(match request.max_selection() {
            Some(max) => NSInteger::try_from(max).unwrap_or(NSInteger::MAX),
            None => 0,
        });
        configuration.setFilter(Some(&filter(request)));
        configuration
    };
    let picker = PhotoPicker::init_with_configuration(mtm.alloc(), &configuration);

    let (completer, completion) = completion::<PickResult<Vec<PathBuf>>>();
    let delegate = Delegate::new(mtm, completer);
    picker.set_delegate(Some(ProtocolObject::from_ref(&*delegate)));
    presenter.presentViewController_animated_completion(&picker, true, None);

    let mut presented = Presented {
        picker,
        _delegate: delegate,
        finished: false,
    };
    Box::pin(async move {
        let paths = completion.await.map_err(|_| PickError::Interrupted)??;
        presented.finish();
        Ok(paths.into_iter().map(PickedFile::from_path).collect())
    })
}

/// The system filter for the requested media.
fn filter(request: &ImagePicker) -> Retained<PHPickerFilter> {
    // SAFETY: class methods that return autoreleased system filters.
    unsafe {
        let media = request.media_kind();
        match (media.includes_images(), media.includes_videos()) {
            (true, true) => PHPickerFilter::anyFilterMatchingSubfilters(&NSArray::from_retained_slice(&[
                PHPickerFilter::imagesFilter(),
                PHPickerFilter::videosFilter(),
            ])),
            (false, true) => PHPickerFilter::videosFilter(),
            _ => PHPickerFilter::imagesFilter(),
        }
    }
}

/// The picker while it is up, with the delegate UIKit only holds weakly.
struct Presented {
    picker: Retained<PhotoPicker>,
    _delegate: Retained<Delegate>,
    finished: bool,
}

impl Presented {
    /// Marks the picker as closed. A method rather than a field write, so the
    /// future captures the whole guard and not just the flag.
    fn finish(&mut self) {
        self.finished = true;
    }
}

impl Drop for Presented {
    fn drop(&mut self) {
        if !self.finished {
            self.picker.dismissViewControllerAnimated_completion(true, None);
        }
    }
}

struct Ivars {
    /// Taken by the first callback; a second callback finds nothing to complete.
    completer: RefCell<Option<Completer<PickResult<Vec<PathBuf>>>>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and the class does not
    // implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AimerPhotoPickerDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl PhotoPickerDelegate for Delegate {
        #[unsafe(method(picker:didFinishPicking:))]
        unsafe fn picker_did_finish_picking(&self, picker: &PhotoPicker, results: &NSArray<PHPickerResult>) {
            // The picker leaves dismissing itself to its delegate.
            picker.dismissViewControllerAnimated_completion(true, None);
            let Some(completer) = self.ivars().completer.borrow_mut().take() else {
                return;
            };
            // SAFETY: a plain getter on a result the picker just handed over.
            let providers: Vec<_> = results.iter().map(|result| unsafe { result.itemProvider() }).collect();
            if providers.is_empty() {
                completer.complete(Ok(Vec::new()));
                return;
            }

            let gather = Arc::new(Gather {
                completer: Mutex::new(Some(completer)),
                slots: Mutex::new(vec![None; providers.len()]),
                remaining: AtomicUsize::new(providers.len()),
            });
            for (index, provider) in providers.iter().enumerate() {
                let type_identifier = if provider.hasItemConformingToTypeIdentifier(&NSString::from_str("public.image")) {
                    "public.image"
                } else {
                    "public.movie"
                };
                let gather = Arc::clone(&gather);
                // Runs once per item on a background queue. The file at `url` is
                // deleted as soon as this returns, so it is copied out here.
                let handler = RcBlock::new(move |url: *mut NSURL, _error: *mut NSError| {
                    // SAFETY: the system passes a valid URL or null.
                    let copied = unsafe { url.as_ref() }.and_then(copy_out);
                    gather.deliver(index, copied);
                });
                // SAFETY: the handler matches the declared block signature and is
                // copied by the framework, which keeps it alive until it runs.
                let _progress = unsafe {
                    provider.loadFileRepresentationForTypeIdentifier_completionHandler(
                        &NSString::from_str(type_identifier),
                        &handler,
                    )
                };
            }
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker, completer: Completer<PickResult<Vec<PathBuf>>>) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(Ivars {
            completer: RefCell::new(Some(completer)),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// Collects the per-item results, which arrive on background queues in any
/// order, and completes once the last one is in.
struct Gather {
    completer: Mutex<Option<Completer<PickResult<Vec<PathBuf>>>>>,
    /// One slot per chosen item, in the user's selection order.
    slots: Mutex<Vec<Option<PathBuf>>>,
    remaining: AtomicUsize,
}

impl Gather {
    fn deliver(&self, index: usize, path: Option<PathBuf>) {
        // Nothing runs under these locks but plain moves, so a poisoned lock is
        // still consistent.
        self.slots.lock().unwrap_or_else(PoisonError::into_inner)[index] = path;
        if self.remaining.fetch_sub(1, Ordering::AcqRel) != 1 {
            return;
        }
        let slots = std::mem::take(&mut *self.slots.lock().unwrap_or_else(PoisonError::into_inner));
        let total = slots.len();
        let paths: Vec<PathBuf> = slots.into_iter().flatten().collect();
        let result = if paths.is_empty() && total > 0 {
            Err(PickError::Platform("the chosen items could not be loaded".into()))
        } else {
            Ok(paths)
        };
        if let Some(completer) = self.completer.lock().unwrap_or_else(PoisonError::into_inner).take() {
            completer.complete(result);
        }
    }
}

static NEXT_COPY: AtomicUsize = AtomicUsize::new(1);

/// Copies the provider's temporary file somewhere it survives, keeping its name.
fn copy_out(url: &NSURL) -> Option<PathBuf> {
    let source = PathBuf::from(url.path()?.to_string());
    let name = Path::new(source.file_name()?);
    // One directory per copy, so two items with the same name never collide.
    let directory = std::env::temp_dir().join("aimer_image_picker").join(format!(
        "{}-{}",
        std::process::id(),
        NEXT_COPY.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).ok()?;
    let destination = directory.join(name);
    std::fs::copy(&source, &destination).ok()?;
    Some(destination)
}

/// The view controller a new sheet should be presented from: the top of the
/// presentation chain on the key window of the foreground scene.
fn top_view_controller(mtm: MainThreadMarker) -> Option<Retained<UIViewController>> {
    let app = UIApplication::sharedApplication(mtm);
    let mut fallback = None;
    for scene in app.connectedScenes().iter() {
        let Ok(scene) = scene.downcast::<UIWindowScene>() else {
            continue;
        };
        let window = scene.keyWindow().or_else(|| scene.windows().firstObject());
        let Some(root) = window.and_then(|window| window.rootViewController()) else {
            continue;
        };
        if scene.activationState() == UISceneActivationState::ForegroundActive {
            return Some(topmost(root));
        }
        fallback.get_or_insert(root);
    }
    fallback.map(topmost)
}

fn topmost(mut controller: Retained<UIViewController>) -> Retained<UIViewController> {
    while let Some(presented) = controller.presentedViewController() {
        controller = presented;
    }
    controller
}
