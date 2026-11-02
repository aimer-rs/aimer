//! Receiving files dragged in from the operating system.
//!
//! [`DropZone`] is the file-drag counterpart of [`DragTarget`]: the same hover
//! state, the same routed hit test, a different source. What makes it its own
//! type is the shape of the platform's events. A drag of five files arrives as
//! five separate hovers and five separate drops, with no marker saying the
//! batch has ended, and an application that wanted the five paths together
//! would otherwise have to reassemble them itself.
//!
//! The reassembly happens here, and it needs no frame hook. Each drop appends
//! its path to a buffer and queues a state mutation; the mutation queue is
//! drained once per frame, so the first mutation to run finds all five paths
//! and delivers them, and the remaining four find an empty buffer and do
//! nothing.
//!
//! # Following the drag
//!
//! The platform announces the drag *entering* the window and then says nothing
//! more, however far the user goes on moving it, so the windowing layer
//! continues the story itself as [`ElementEvent::HoveredFileMoved`]: one
//! hit-tested event per position, carrying the whole batch. A zone therefore
//! lights up as the files reach it and goes dark as they leave, exactly like a
//! [`DragTarget`] under a pointer. Both events are handled the same way here,
//! which is why recording a path is idempotent — the drag in flight is reported
//! over and over, and the batch must not grow with it.
//!
//! A drag between two zones is over none of them, and a hit test has nobody to
//! answer it, so that one case arrives as a broadcast
//! [`ElementEvent::DragLeave`]: the highlight goes, while the collected paths
//! and the session stay, because the drag may still come back before it is
//! dropped.
//!
//! # Platforms
//!
//! winit's web backend emits no file-drag events at all, so a `DropZone`
//! compiles for the browser and never fires there.
//!
//! [`DragTarget`]: crate::DragTarget

use std::cell::RefCell;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_events::element::ElementEvent;
use aimer_events::pointer::{FILE_DRAG_POINTER_ID, PointerSource};
use aimer_widget::InteractionBounds;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, EventResult, EventTreeRole,
    LayoutElement, PointerKey, Rebuildable, RequiredChild, State, StateUpdater, StatefulElement,
    StatefulWidget, VisitorElement, Widget,
};

use crate::target::{DragTargetState, HasChild, TargetChild, clear_hover, enter_hover};
use crate::{DragPayload, DragSession, FileDrop};

/// The pointer a file drag is filed under.
///
/// An operating-system drag has no pointer of its own — there is no press to
/// own — but the session is keyed by one, so file drags get a reserved
/// identifier that no real device can collide with.
const FILE_DRAG_POINTER: PointerKey = PointerKey::new(PointerSource::Mouse, FILE_DRAG_POINTER_ID);

/// Receives every path of one file drag, in one call.
type DropHandler = Rc<dyn Fn(Vec<PathBuf>)>;

/// A region that accepts files dragged in from the operating system.
///
/// `new()` takes nothing and [`DropZone::child`] comes last.
///
/// # Examples
///
/// ```no_run
/// use std::path::PathBuf;
///
/// use aimer_container::{Container, ZeroSizedBox};
/// use aimer_dnd::{DragTargetState, DropZone};
///
/// let uploads = DropZone::new()
///     .extensions(["png", "jpg", "jpeg"])
///     .on_drop(|paths: Vec<PathBuf>| {
///         // Called once per drag, with every matching file in it.
///         let _count = paths.len();
///     })
///     .child(|state: DragTargetState| {
///         let _highlight = state.is_hovered;
///         Container::new().child(ZeroSizedBox)
///     });
/// ```
#[derive(aimer_widget::PortableWidget)]
#[portable_widget(id = "aimer_dnd::DropZone", schema_only)]
pub struct DropZone<C = RequiredChild> {
    #[portable_skip]
    extensions: Option<Rc<[String]>>,
    #[portable_skip]
    on_drop: Option<DropHandler>,
    // The child is rebuilt from live hover state, so it has no single static
    // portable child node. Keep the callback on the native path.
    #[portable_skip]
    child: Option<TargetChild>,
    #[portable_skip]
    _child: PhantomData<C>,
}

impl Default for DropZone {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl DropZone {
    /// Creates an incomplete zone that accepts every file.
    #[inline]
    pub fn new() -> Self {
        Self {
            extensions: None,
            on_drop: None,
            child: None,
            _child: PhantomData,
        }
    }
}

impl<C> DropZone<C> {
    /// Restricts the zone to files with one of these extensions.
    ///
    /// Matching ignores case and ignores a leading dot, so `"png"` and `".PNG"`
    /// mean the same thing. A path with no extension never matches a restricted
    /// zone, and a drag that does not match neither highlights the zone nor
    /// reaches [`DropZone::on_drop`].
    #[inline]
    pub fn extensions<I, S>(mut self, extensions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.extensions = Some(
            extensions
                .into_iter()
                .map(|extension| {
                    extension
                        .as_ref()
                        .trim_start_matches('.')
                        .to_ascii_lowercase()
                })
                .collect(),
        );
        self
    }

    /// Receives every matching path of one drag, in one call.
    #[inline]
    pub fn on_drop<F: Fn(Vec<PathBuf>) + 'static>(mut self, on_drop: F) -> Self {
        self.on_drop = Some(Rc::new(on_drop));
        self
    }

    /// Builds the zone's content from its hover state, completing the builder.
    #[inline]
    pub fn child<F, W>(self, child: F) -> DropZone<HasChild>
    where
        F: Fn(DragTargetState) -> W + 'static,
        W: Widget + 'static,
    {
        DropZone {
            extensions: self.extensions,
            on_drop: self.on_drop,
            child: Some(Rc::new(move |state| child(state).boxed())),
            _child: PhantomData,
        }
    }
}

/// Whether `path` is one of the extensions a zone was restricted to.
///
/// `None` means the zone was not restricted and takes everything.
fn matches_extensions(extensions: Option<&[String]>, path: &Path) -> bool {
    let Some(extensions) = extensions else {
        return true;
    };
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extensions.contains(&extension.to_ascii_lowercase()))
}

impl Widget for DropZone<HasChild> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::from_widget(self, ctx, "DropZone", None)
    }

    fn debug_name(&self) -> &'static str {
        "DropZone"
    }
}

/// The state one zone keeps between frames.
pub struct DropZoneLiveState {
    id: u64,
    hover: DragTargetState,
    extensions: Option<Rc<[String]>>,
    on_drop: Option<DropHandler>,
    child: Option<TargetChild>,
    /// The paths of the drag being received, filled as the platform reports
    /// them and drained by the first queued mutation to run.
    pending: Rc<RefCell<Vec<PathBuf>>>,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for DropZone<HasChild> {
    type State = DropZoneLiveState;

    fn create_state(self) -> Self::State {
        DropZoneLiveState {
            id: crate::target::next_target_id(),
            hover: DragTargetState::default(),
            extensions: self.extensions.clone(),
            on_drop: self.on_drop.clone(),
            child: self.child.clone(),
            pending: Rc::new(RefCell::new(Vec::new())),
            updater: StateUpdater::empty(),
        }
    }
}

impl State<DropZone<HasChild>> for DropZoneLiveState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn adopt_config_from(&mut self, new: Self) {
        self.extensions = new.extensions;
        self.on_drop = new.on_drop;
        self.child = new.child;
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let content = self
            .child
            .as_ref()
            .map(|build| build(self.hover))
            .unwrap_or_else(|| Widget::boxed(aimer_container::ZeroSizedBox));

        DropZoneGate {
            child: content,
            logic: Rc::new(ZoneLogic {
                id: self.id,
                extensions: self.extensions.clone(),
                pending: self.pending.clone(),
                updater: self.updater,
            }),
        }
    }
}

/// The event-handling half of a zone.
struct ZoneLogic {
    id: u64,
    extensions: Option<Rc<[String]>>,
    pending: Rc<RefCell<Vec<PathBuf>>>,
    updater: StateUpdater<DropZoneLiveState>,
}

impl ZoneLogic {
    fn accepts(&self, path: &Path) -> bool {
        matches_extensions(self.extensions.as_deref(), path)
    }

    fn set_hover(&self, hover: DragTargetState) {
        if self.updater.read(|state| state.hover) == hover {
            return;
        }
        self.updater.set_state(move |state| state.hover = hover);
    }

    /// Queues the delivery of whatever has been collected so far.
    ///
    /// Queued once per dropped file. The queue drains once per frame, so the
    /// first of them takes every path and calls back; the rest find nothing.
    fn queue_delivery(&self) {
        self.updater.set_state(|state| {
            let paths = std::mem::take(&mut *state.pending.borrow_mut());
            if paths.is_empty() {
                return;
            }
            state.hover = DragTargetState::default();
            if let Some(on_drop) = state.on_drop.clone() {
                on_drop(paths);
            }
        });
    }

    /// Forgets the drag: no highlight, no collected paths, no session.
    fn abandon(&self) {
        self.pending.borrow_mut().clear();
        DragSession::cancel(FILE_DRAG_POINTER);
        self.set_hover(DragTargetState::default());
    }
}

struct DropZoneGate {
    child: AnyWidget,
    logic: Rc<ZoneLogic>,
}

impl Widget for DropZoneGate {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawDropZone {
            child: self.child.to_element(ctx),
            logic: self.logic.clone(),
            bounds: InteractionBounds::new(),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "DropZoneGate"
    }
}

impl aimer_widget::PortableWidget for DropZoneGate {}

struct RawDropZone {
    child: AnyElement,
    logic: Rc<ZoneLogic>,
    bounds: InteractionBounds,
}

/// Appends `path` unless the batch already carries it.
///
/// The drag in flight is reported again for every position it moves through, so
/// recording has to be idempotent: without this a batch grows by one copy of
/// itself per pixel travelled, and a target reading the session sees the same
/// file a hundred times over.
fn append_path(paths: &mut Vec<PathBuf>, path: &Path) {
    if !paths.iter().any(|known| known == path) {
        paths.push(path.to_path_buf());
    }
}

impl RawDropZone {
    /// Opens the shared drag session for this file drag, or adds to the one
    /// already open.
    fn record(&self, path: &Path) {
        let extended = DragSession::with_payload_mut(|drop: &mut FileDrop| {
            append_path(&mut drop.paths, path);
        });
        if extended.is_none() {
            DragSession::begin(
                FILE_DRAG_POINTER,
                DragPayload::new(FileDrop::new(vec![path.to_path_buf()])),
                Vec2d::default(),
            );
        }
    }

    /// Takes the drag if any of its files is one this zone wants.
    ///
    /// A mixed batch dropped on a restricted zone delivers the files it asked
    /// for and silently leaves the rest, so a batch is worth highlighting for as
    /// long as one file in it would be kept.
    fn take_batch(&self, paths: &[PathBuf]) -> EventResult {
        let mut wanted = paths
            .iter()
            .filter(|path| self.logic.accepts(path))
            .peekable();
        if wanted.peek().is_none() {
            return EventResult::ignored();
        }
        for path in wanted {
            self.record(path);
        }
        self.enter();
        EventResult::consumed().with_redraw()
    }

    fn enter(&self) {
        let logic = self.logic.clone();
        enter_hover(
            self.logic.id,
            Rc::new(move || logic.set_hover(DragTargetState::default())),
        );
        self.logic.set_hover(DragTargetState {
            is_hovered: true,
            will_accept: true,
        });
    }
}

impl VisitorElement for RawDropZone {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "DropZone"
    }
}

impl EventElement for RawDropZone {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedTarget
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        match event {
            ElementEvent::HoveredFile { path, .. } => self.take_batch(std::slice::from_ref(path)),

            ElementEvent::HoveredFileMoved { paths, .. } => self.take_batch(paths),

            ElementEvent::HoveredFileCancelled => {
                // Broadcast, not hit-tested: the cursor may already have left
                // the zone that lit up.
                self.logic.abandon();
                clear_hover();
                EventResult::ignored()
            }

            ElementEvent::DragLeave { .. } => {
                // The drag is still over the window, just not over any zone, so
                // only the highlight goes: the collected paths and the session
                // stay, because the drag may well come back before it is
                // dropped.
                clear_hover();
                EventResult::ignored()
            }

            ElementEvent::DroppedFile { path, .. } => {
                if !self.logic.accepts(path) {
                    return EventResult::ignored();
                }
                self.logic.pending.borrow_mut().push(path.to_path_buf());
                DragSession::cancel(FILE_DRAG_POINTER);
                clear_hover();
                self.logic.queue_delivery();
                EventResult::consumed().with_redraw()
            }

            ElementEvent::Cancel => {
                self.logic.abandon();
                EventResult::ignored()
            }

            _ => EventResult::ignored(),
        }
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl LayoutElement for RawDropZone {
    #[inline]
    fn size(&self) -> Option<Size> {
        None
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        let size = self.child.layout(ctx);
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        self.bounds
            .save(ctx.scale, abs_x, abs_y, size.width, size.height);
        size
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.bounds.pos_start_end()
    }
}

impl Drawable for RawDropZone {
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    /// Hit-tested against the rectangle this element reports as its own size,
    /// which can be larger than the content box its render node is laid out as.
    #[inline]
    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        Some(self.child.computed_size(ctx))
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.bounds.adopt(source);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<aimer_widget::InteractionDisagreement> {
        self.bounds.disagreement("DropZone")
    }

    fn update(&self, ctx: &BuildContext) {
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        let size = self.child.computed_size(ctx);
        self.bounds
            .save(ctx.scale, abs_x, abs_y, size.width, size.height);
        self.child.update(ctx);
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        let size = self.child.computed_size(ctx);
        self.bounds
            .save(ctx.scale, abs_x, abs_y, size.width, size.height);
        self.child.sync_paint_geometry(ctx);
    }
}

impl Rebuildable for RawDropZone {}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestElement;

    impl VisitorElement for TestElement {
        fn debug_name(&self) -> &'static str {
            "TestElement"
        }
    }

    impl EventElement for TestElement {}
    impl LayoutElement for TestElement {}
    impl Rebuildable for TestElement {}

    impl Drawable for TestElement {
        fn update(&self, _ctx: &BuildContext) {}
    }

    #[test]
    fn drop_zone_opts_into_indexed_event_routing() {
        let zone = RawDropZone {
            child: TestElement.boxed(),
            logic: Rc::new(ZoneLogic {
                id: 1,
                extensions: None,
                pending: Rc::new(RefCell::new(Vec::new())),
                updater: StateUpdater::empty(),
            }),
            bounds: InteractionBounds::new(),
        };

        assert_eq!(zone.event_tree_role(), EventTreeRole::IndexedTarget);
    }

    fn allowed() -> Vec<String> {
        vec!["png".to_owned(), "jpg".to_owned()]
    }

    #[test]
    fn an_unrestricted_zone_takes_everything() {
        assert!(matches_extensions(None, Path::new("notes")));
        assert!(matches_extensions(None, Path::new("archive.tar.gz")));
    }

    #[test]
    fn extensions_match_regardless_of_case() {
        let allowed = allowed();

        assert!(matches_extensions(Some(&allowed), Path::new("a.PNG")));
        assert!(matches_extensions(Some(&allowed), Path::new("a.Jpg")));
        assert!(!matches_extensions(Some(&allowed), Path::new("a.gif")));
    }

    #[test]
    fn a_path_without_an_extension_never_matches_a_restricted_zone() {
        assert!(!matches_extensions(Some(&allowed()), Path::new("README")));
    }

    #[test]
    fn a_path_is_recorded_once_however_often_the_drag_is_reported() {
        let mut paths = Vec::new();

        for _ in 0..3 {
            append_path(&mut paths, Path::new("/tmp/a.png"));
        }
        append_path(&mut paths, Path::new("/tmp/b.png"));

        assert_eq!(
            paths,
            [PathBuf::from("/tmp/a.png"), PathBuf::from("/tmp/b.png")]
        );
    }

    #[test]
    fn a_leading_dot_in_the_filter_is_ignored() {
        let zone = DropZone::new().extensions([".PNG", "jpg"]);
        let extensions = zone.extensions.expect("the zone was restricted");

        assert_eq!(&*extensions, ["png".to_owned(), "jpg".to_owned()]);
    }

    #[test]
    fn completed_drop_zone_publishes_a_derived_schema() {
        use aimer_widget::portable::__anteros::ChildCardinality;
        use aimer_widget::portable::PortableWidgetSchema;

        assert_eq!(
            <DropZone<HasChild> as PortableWidgetSchema>::SCHEMA.children(),
            ChildCardinality::none()
        );
    }
}
