//! iOS: `UIDocumentPickerViewController`.
//!
//! The picker opens the Files browser in a sheet over the topmost view
//! controller of the foreground scene. It is created `asCopy`, so every chosen
//! file arrives as a copy inside the app's sandbox: a plain path, readable at
//! once, with no security-scoped access to start and stop. The price is one copy
//! per file; the original stays where the user keeps it.
//!
//! UIKit holds a picker's delegate weakly, so the future owns it. Dropping the
//! future before the picker closes dismisses the picker.

use std::cell::RefCell;
use std::path::PathBuf;

use aimer_venus::{Completer, completion};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSArray, NSObject, NSObjectProtocol, NSString, NSURL};
use objc2_ui_kit::{
    UIApplication, UIDocumentPickerDelegate, UIDocumentPickerViewController, UISceneActivationState,
    UIViewController, UIWindowScene,
};
use objc2_uniform_type_identifiers::UTType;

use crate::apple_types::content_types;
use crate::error::PickError;
use crate::future::{PickFuture, ready};
use crate::picked::PickedFile;
use crate::request::FilePicker;

pub(crate) fn pick(request: FilePicker) -> PickFuture<Vec<PickedFile>> {
    let Some(mtm) = MainThreadMarker::new() else {
        return ready(Err(PickError::Platform(
            "the document picker can only be shown from the main thread".into(),
        )));
    };
    let Some(presenter) = top_view_controller(mtm) else {
        return ready(Err(PickError::NoWindow));
    };

    let mut types = content_types(&request.filters);
    if types.is_empty() {
        // The picker rejects an empty list; `public.item` is "anything".
        types.extend(UTType::typeWithIdentifier(&NSString::from_str("public.item")));
    }
    let picker = UIDocumentPickerViewController::initForOpeningContentTypes_asCopy(
        mtm.alloc(),
        &NSArray::from_retained_slice(&types),
        true,
    );
    picker.setAllowsMultipleSelection(request.multiple);

    let (completer, completion) = completion::<Vec<PathBuf>>();
    let delegate = Delegate::new(mtm, completer);
    picker.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    presenter.presentViewController_animated_completion(&picker, true, None);

    let mut presented = Presented {
        picker,
        _delegate: delegate,
        finished: false,
    };
    Box::pin(async move {
        let paths = completion.await.map_err(|_| PickError::Interrupted)?;
        presented.finish();
        Ok(paths.into_iter().map(PickedFile::from_path).collect())
    })
}

/// The picker while it is up, with the delegate UIKit only holds weakly.
struct Presented {
    picker: Retained<UIDocumentPickerViewController>,
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
    completer: RefCell<Option<Completer<Vec<PathBuf>>>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and the class does not
    // implement `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AimerFilePickerDelegate"]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UIDocumentPickerDelegate for Delegate {
        #[unsafe(method(documentPicker:didPickDocumentsAtURLs:))]
        fn did_pick(&self, _controller: &UIDocumentPickerViewController, urls: &NSArray<NSURL>) {
            let paths = urls
                .iter()
                .filter_map(|url| url.path())
                .map(|path| PathBuf::from(path.to_string()))
                .collect();
            self.finish(paths);
        }

        #[unsafe(method(documentPickerWasCancelled:))]
        fn was_cancelled(&self, _controller: &UIDocumentPickerViewController) {
            self.finish(Vec::new());
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker, completer: Completer<Vec<PathBuf>>) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(Ivars {
            completer: RefCell::new(Some(completer)),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }

    fn finish(&self, paths: Vec<PathBuf>) {
        if let Some(completer) = self.ivars().completer.borrow_mut().take() {
            completer.complete(paths);
        }
    }
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
