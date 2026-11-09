//! The picture of a widget's content, as it is drawn in the window.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use aimer_attribute::{ResolvedSize, Size, Vec2d};
use aimer_canvas::Canvas;
use aimer_platform::capture::{self, Error as WindowError};
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, EventTreeRole, LayoutElement, Rebuildable,
    RequiredChild, RetainedNodeHandle, VisitorElement, Widget,
};

use crate::Image;

/// A rectangle of the window, in logical pixels from its top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    /// Distance from the window's left edge.
    pub x: f32,
    /// Distance from the window's top edge.
    pub y: f32,
    /// Width.
    pub width: f32,
    /// Height.
    pub height: f32,
}

impl Region {
    /// A region, or `None` when it has no area or any of its numbers is not
    /// finite: there is no picture of nothing.
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Option<Self> {
        ([x, y, width, height].iter().all(|v| v.is_finite()) && width > 0.0 && height > 0.0)
            .then_some(Self { x, y, width, height })
    }
}

/// Why a picture of a widget's content could not be had.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShotError {
    /// The window itself could not be captured.
    Window(WindowError),
    /// The content is not on screen: it has not been drawn yet, it has no area,
    /// or it has left the widget tree.
    NotShown,
    /// The content is entirely outside the window, say scrolled out of view.
    OffScreen,
}

impl fmt::Display for ShotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Window(error) => error.fmt(f),
            Self::NotShown => f.write_str("the content is not on screen"),
            Self::OffScreen => f.write_str("the content is outside the window"),
        }
    }
}

impl std::error::Error for ShotError {}

impl From<WindowError> for ShotError {
    fn from(error: WindowError) -> Self {
        Self::Window(error)
    }
}

/// A picture of some content that has been asked for and is on its way.
///
/// It holds no widget state, so it can be handed to another thread to wait on:
/// the frame it needs is drawn by the renderer, and waiting for it on the thread
/// that draws would wait for itself.
pub struct CaptureRequest {
    region: Region,
    reply: Receiver<capture::Reply>,
    done: bool,
}

impl CaptureRequest {
    pub(crate) fn new(region: Region, reply: Receiver<capture::Reply>) -> Self {
        Self { region, reply, done: false }
    }

    /// The part of the window this will be a picture of.
    #[inline]
    pub fn region(&self) -> Region {
        self.region
    }

    /// Waits for the frame and returns the content's picture. Not on the thread
    /// that draws the window.
    ///
    /// # Errors
    ///
    /// [`ShotError::Window`] when the window could not be captured or no frame
    /// came within `timeout`, and [`ShotError::OffScreen`] when the content is
    /// outside the window.
    pub fn wait(self, timeout: Duration) -> Result<Image, ShotError> {
        match self.reply.recv_timeout(timeout) {
            Ok(reply) => self.finish(reply),
            Err(_) => Err(ShotError::Window(WindowError::TimedOut)),
        }
    }

    /// The picture if the frame has arrived, and `None` if it has not — or if the
    /// answer has already been given. Never blocks, so the thread that draws can
    /// ask each frame.
    pub fn poll(&mut self) -> Option<Result<Image, ShotError>> {
        if self.done {
            return None;
        }
        match self.reply.try_recv() {
            Ok(reply) => {
                self.done = true;
                Some(self.finish(reply))
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.done = true;
                Some(Err(ShotError::Window(WindowError::TimedOut)))
            }
        }
    }

    fn finish(&self, reply: capture::Reply) -> Result<Image, ShotError> {
        let frame = reply?;
        let Region { x, y, width, height } = self.region;
        Image::from_frame(&frame).crop_logical(x, y, width, height).ok_or(ShotError::OffScreen)
    }
}

#[derive(Default)]
struct Registration {
    node: Option<RetainedNodeHandle>,
    /// Which element registered, so that a replaced element dropping later does
    /// not clear the registration that replaced it.
    owner: u64,
}

/// Where a [`Capture`]'s content is, and how to ask for a picture of it.
///
/// Make one, give a clone to [`Capture::handle`], and keep the other to ask with.
/// It belongs to the thread that draws the widget tree, like the tree does.
#[derive(Clone, Default)]
pub struct CaptureHandle(Rc<RefCell<Registration>>);

impl CaptureHandle {
    /// A handle that is not attached to anything yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Where the content is in the window right now, or `None` if it is not on
    /// screen. Read when asked, so it follows scrolling and layout.
    pub fn bounds(&self) -> Option<Region> {
        let (x, y, width, height) = self.0.borrow().node.as_ref()?.world_bounds()?;
        Region::new(x, y, width, height)
    }

    /// Asks for a picture of the content as it is drawn in the next frame.
    ///
    /// The region is read now, on the thread that draws; the picture arrives
    /// when the renderer has drawn a frame, which this wakes it to do. Hand the
    /// request to a worker to [`wait`](CaptureRequest::wait) on, or
    /// [`poll`](CaptureRequest::poll) it each frame.
    ///
    /// # Errors
    ///
    /// [`ShotError::NotShown`] when the content is not on screen.
    pub fn request(&self) -> Result<CaptureRequest, ShotError> {
        self.request_from(capture::request_and_wake())
    }

    /// [`CaptureHandle::request`], with the frame cut from whatever answers
    /// `reply` instead of the window's renderer. Lets a test stand in for one.
    #[doc(hidden)]
    pub fn request_from(&self, reply: Receiver<capture::Reply>) -> Result<CaptureRequest, ShotError> {
        let region = self.bounds().ok_or(ShotError::NotShown)?;
        Ok(CaptureRequest::new(region, reply))
    }
}

thread_local! {
    static OWNERS: Cell<u64> = const { Cell::new(0) };
}

fn next_owner() -> u64 {
    OWNERS.with(|owners| {
        owners.set(owners.get() + 1);
        owners.get()
    })
}

/// Makes its child's content something that can be turned into an image.
///
/// The widget takes no space and draws nothing of its own: it lays out and
/// paints as its child, and lets a [`CaptureHandle`] find where the child is.
/// Ask the handle for a [`CaptureRequest`] and the picture is that part of the
/// window as it is drawn — everything overlapping the child included, and
/// whatever is clipped away left out — which is what a person sees there.
///
/// # Examples
///
/// ```
/// use aimer_capture::{Capture, CaptureHandle};
/// use aimer_widget::ErrorWidget;
///
/// let handle = CaptureHandle::new();
/// let card = Capture::new().handle(&handle).child(ErrorWidget::new("card"));
/// // Later, on the thread that draws the tree:
/// // let request = handle.request()?;  // then hand it to a worker to wait on
/// # let _ = card;
/// ```
pub struct Capture<W = RequiredChild> {
    handle: Option<CaptureHandle>,
    child: W,
}

impl Default for Capture {
    fn default() -> Self {
        Self::new()
    }
}

impl Capture {
    /// Creates a builder that requires a child.
    ///
    /// Finish it with [`Capture::child`] or [`Capture::box_child`].
    #[inline]
    pub fn new() -> Self {
        Self { handle: None, child: RequiredChild }
    }
}

impl<W> Capture<W> {
    /// Attaches `handle`, which then reports where the content is and asks for
    /// its picture.
    #[inline]
    pub fn handle(mut self, handle: &CaptureHandle) -> Self {
        self.handle = Some(handle.clone());
        self
    }

    /// Attaches the required child and completes this builder.
    #[inline]
    pub fn child<C: Widget>(self, child: C) -> Capture<C> {
        Capture { handle: self.handle, child }
    }

    /// Attaches `child` and erases the resulting widget's concrete type.
    #[inline]
    pub fn box_child<C: Widget + 'static>(self, child: C) -> AnyWidget {
        self.child(child).boxed()
    }
}

impl<W: Widget> Widget for Capture<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawCapture { handle: self.handle, child: self.child.to_element(ctx), owner: next_owner() }.boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Capture"
    }
}

struct RawCapture {
    handle: Option<CaptureHandle>,
    child: AnyElement,
    owner: u64,
}

impl Drop for RawCapture {
    fn drop(&mut self) {
        if let Some(handle) = &self.handle {
            let mut registration = handle.0.borrow_mut();
            if registration.owner == self.owner {
                registration.node = None;
            }
        }
    }
}

impl Rebuildable for RawCapture {}

impl Drawable for RawCapture {
    fn update(&self, ctx: &BuildContext) {
        self.child.update(ctx);
    }

    #[inline]
    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.child.sync_paint_geometry(ctx);
    }

    /// The element owns a render node of its own, with nothing recorded on it, so
    /// there is a node whose bounds are the child's.
    #[inline]
    fn can_paint(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint(&self, ctx: &BuildContext) {
        Canvas::of(ctx).finish();
    }

    fn sync_paint_state(&self, ctx: &BuildContext) -> bool {
        if let (Some(handle), Some(node)) = (&self.handle, ctx.local_v2_node_handle()) {
            let mut registration = handle.0.borrow_mut();
            registration.node = Some(node);
            registration.owner = self.owner;
        }
        true
    }
}

impl EventElement for RawCapture {
    #[inline]
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }

    #[inline]
    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl VisitorElement for RawCapture {
    #[inline]
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "Capture"
    }
}

impl LayoutElement for RawCapture {
    fn size(&self) -> Option<Size> {
        self.child.size()
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.layout(ctx)
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.content_size(ctx)
    }

    fn layer(&self) -> u32 {
        self.child.layer()
    }

    fn flex(&self) -> Option<f32> {
        self.child.flex()
    }

    #[inline]
    fn is_layout_stable(&self) -> bool {
        self.child.is_layout_stable()
    }

    fn get_size_from_child(&self) -> Option<Size> {
        self.child.get_size_from_child()
    }

    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.child.pos_start_end()
    }
}

#[cfg(test)]
#[path = "region_tests.rs"]
mod tests;
