use std::cell::Cell;
use std::marker::PhantomData;
use std::sync::OnceLock;
use std::time::Duration;

use aimer_animation::{AnimInstant, AnimationController, AnyCurve, Curve, RotationTransition, curves};
use aimer_container::Container;
use aimer_events::element::ElementEvent;
use aimer_events::window::request_animation_frame;
use aimer_flex::{BoxAlignment, Column, Expanded, Row};
use aimer_style::{LayoutSpacing, ThemeData};
use aimer_svg::{Svg, SvgDocument, SvgStyle};
use aimer_widget::InteractionBounds;
use aimer_widget::base::{BuildContext, ResolvedSize, Vec2d};
use aimer_widget::{
    AnyElement, AnyWidget, ChildBuilder, Drawable, Element, EventElement, EventResult,
    EventTreeRole, LayoutElement, PortableWidget, Rebuildable, RequiredChild, State, StateUpdater,
    StatefulElement, StatefulWidget, VisitorElement, Widget,
};

use crate::key_relay::KeyRelay;
use aimer_input::gesture::gesture_detector::{GestureDetector, GestureDetectorBehavior};
use aimer_provider::ProviderContext;

const CHEVRON_ICON: &str = include_str!("../icons/chevron-down-svgrepo-com.svg");
const COLLAPSIBLE_ANIMATION_MILLIS: u64 = 180;

fn chevron_document() -> SvgDocument {
    static DOCUMENT: OnceLock<SvgDocument> = OnceLock::new();

    DOCUMENT
        .get_or_init(|| {
            SvgDocument::from_svg(CHEVRON_ICON.as_bytes())
                .expect("the bundled collapsible-list chevron SVG should be valid")
        })
        .clone()
}

fn chevron(theme: ThemeData, controller: AnimationController) -> AnyWidget {
    Container::new()
        // .color(Color::WHITE)
        .width(24.0)
        .height(24.0)
        .box_child(
            RotationTransition::new(
                controller,
                Svg::new(chevron_document())
                    .style("#icon-path", SvgStyle::new().stroke(theme.primary_color)),
            )
            .turn_range(0.0, -0.25),
        )
}

#[inline]
fn animation_progress(value: f32) -> f32 {
    value
        .is_finite()
        .then_some(value.clamp(0.0, 1.0))
        .unwrap_or(0.0)
}

#[inline]
fn nonnegative_extent(value: f32) -> f32 {
    value.is_finite().then_some(value.max(0.0)).unwrap_or(0.0)
}

#[doc(hidden)]
#[inline]
pub(crate) fn collapsed_height(natural_height: f32, progress: f32) -> f32 {
    nonnegative_extent(natural_height) * animation_progress(progress)
}

/// Retained layout wrapper used by [`CollapsibleList`] to animate its body's
/// vertical extent while clipping the body to that extent.
struct AnimatedCollapse<T: Widget + 'static> {
    controller: AnimationController,
    child: T,
}

impl<T: Widget> AnimatedCollapse<T> {
    #[inline]
    fn new(controller: AnimationController, child: T) -> Self {
        Self { controller, child }
    }
}

impl<T: Widget + 'static> Widget for AnimatedCollapse<T> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        AnimatedCollapseElement {
            child: self.child.to_element(ctx),
            controller: self.controller,
            last_progress: Cell::new(None),
            bounds: InteractionBounds::new(),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "AnimatedCollapse"
    }
}

impl<T: Widget + 'static> PortableWidget for AnimatedCollapse<T> {}

struct AnimatedCollapseElement {
    child: AnyElement,
    controller: AnimationController,
    last_progress: Cell<Option<u32>>,
    bounds: InteractionBounds,
}

// SAFETY: Aimer renders and mutates retained elements on one UI thread.
unsafe impl Send for AnimatedCollapseElement {}
unsafe impl Sync for AnimatedCollapseElement {}

impl AnimatedCollapseElement {
    #[inline]
    fn progress(&self) -> f32 {
        animation_progress(self.controller.tick(AnimInstant::now()))
    }

    #[inline]
    fn is_event_visible(&self) -> bool {
        animation_progress(self.controller.value()) > 0.0
    }

    fn natural_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let mut natural_ctx = ctx.clone();
        natural_ctx.box_constraint.min_height = 0.0;
        natural_ctx.box_constraint.max_height = f32::MAX;
        natural_ctx.parent_size.height = f32::MAX;
        natural_ctx.visible_rect = None;
        self.child.computed_size(&natural_ctx)
    }

    #[inline]
    fn child_context<'a>(&self, ctx: &BuildContext<'a>, natural: ResolvedSize) -> BuildContext<'a> {
        let mut child_ctx = ctx.clone();
        child_ctx.parent_size = natural;
        child_ctx.box_constraint.min_width = 0.0;
        child_ctx.box_constraint.min_height = 0.0;
        child_ctx.box_constraint.max_width = natural.width;
        child_ctx.box_constraint.max_height = natural.height;
        child_ctx
    }

    /// The size the body currently occupies: its natural width and a height
    /// that follows the animation.
    fn current_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let natural = self.natural_size(ctx);
        ResolvedSize {
            width: nonnegative_extent(natural.width),
            height: collapsed_height(natural.height, self.progress()),
        }
    }

    /// Schedules the next frame's layout after this frame sampled `progress`.
    ///
    /// The retained render tree re-reads bounds only when the layout
    /// invalidation generation moves, and this body's extent follows the
    /// controller rather than any rebuild. Without a bump here the body, its
    /// clip, and every sibling placed after it keep the size of the last
    /// synchronization until something else invalidates layout, so the
    /// transition would jump instead of animate. A frame is also requested
    /// after the sample that settles the animation: the tree was synchronized
    /// from an earlier sample than the one that finished it.
    ///
    /// Returns whether the body's extent changed or is still changing.
    fn advance_layout(&self, progress: f32) -> bool {
        let bits = progress.to_bits();
        let moved = matches!(self.last_progress.replace(Some(bits)), Some(last) if last != bits);
        let active = self.controller.is_animating();
        if moved || active {
            self.child.invalidate_layout();
            request_animation_frame();
        }
        moved || active
    }

    fn update_bounds(&self, ctx: &BuildContext, size: ResolvedSize) {
        let scale = ctx.scale;
        if !scale.is_finite() || scale <= 0.0 {
            return;
        }

        let width = size.width.max(0.0);
        let height = size.height.max(0.0);
        if !width.is_finite() || !height.is_finite() {
            return;
        }

        let (start_x, start_y) = ctx.canvas.get_transform_translation();
        self.bounds.save(scale, start_x, start_y, width, height);
    }
}

impl Drawable for AnimatedCollapseElement {
    /// Hit-tested over the body as it currently occupies space.
    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        Some(self.current_size(ctx))
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.bounds.adopt(source);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<aimer_widget::InteractionDisagreement> {
        self.bounds.disagreement("AnimatedCollapse")
    }

    fn update(&self, ctx: &BuildContext) {
        let progress = self.progress();
        let natural = self.natural_size(ctx);
        let size = ResolvedSize {
            width: nonnegative_extent(natural.width),
            height: collapsed_height(natural.height, progress),
        };
        self.update_bounds(ctx, size);

        if self.advance_layout(progress) {
            // The body's height changes the position of every following
            // sibling, so a complete repaint also clears the old footprint.
            aimer_widget::mark_paint_damage_full();
        }

        if size.width > 0.0 && size.height > 0.0 {
            self.child.update(&self.child_context(ctx, natural));
        }
    }

    #[inline]
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    #[inline]
    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    fn sync_local_v2_state(&self, ctx: &BuildContext) -> bool {
        let progress = self.progress();
        let natural = self.natural_size(ctx);
        let size = ResolvedSize {
            width: nonnegative_extent(natural.width),
            height: collapsed_height(natural.height, progress),
        };
        self.update_bounds(ctx, size);
        self.child
            .sync_paint_geometry(&self.child_context(ctx, natural));
        true
    }

    fn draw_local_v2_compatibility(&self, ctx: &BuildContext) {
        let progress = self.progress();
        let natural = self.natural_size(ctx);
        let size = ResolvedSize {
            width: nonnegative_extent(natural.width),
            height: collapsed_height(natural.height, progress),
        };
        self.update_bounds(ctx, size);
        self.advance_layout(progress);
        if size.width > 0.0 && size.height > 0.0 {
            self.child.update(&self.child_context(ctx, natural));
        }
    }

    fn retained_clip(&self, ctx: &BuildContext) -> Option<aimer_cupid::draw_cmd_v2::Rect> {
        let natural = self.natural_size(ctx);
        let size = ResolvedSize {
            width: nonnegative_extent(natural.width),
            height: collapsed_height(natural.height, self.progress()),
        };
        Some(aimer_cupid::draw_cmd_v2::Rect::new(
            0.0,
            0.0,
            size.width,
            size.height,
        ))
    }

    fn retained_v2_bounds(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        let natural = self.natural_size(ctx);
        Some(ResolvedSize {
            width: nonnegative_extent(natural.width),
            height: collapsed_height(natural.height, self.progress()),
        })
    }

    fn retained_v2_child_context<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
    ) -> Option<BuildContext<'a>> {
        std::ptr::eq(child, self.child.as_ref())
            .then(|| self.child_context(ctx, self.natural_size(ctx)))
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        let progress = animation_progress(self.controller.value());
        let natural = self.natural_size(ctx);
        let size = ResolvedSize {
            width: nonnegative_extent(natural.width),
            height: collapsed_height(natural.height, progress),
        };
        self.update_bounds(ctx, size);
        self.child
            .sync_paint_geometry(&self.child_context(ctx, natural));
    }
}

impl VisitorElement for AnimatedCollapseElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "AnimatedCollapseElement"
    }
}

impl EventElement for AnimatedCollapseElement {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedTarget
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        if self.is_event_visible() {
            self.child.on_event(event)
        } else {
            EventResult::ignored()
        }
    }

    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        if self.is_event_visible() {
            visitor(self.child.as_ref());
        }
    }

    fn focus_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        if self.is_event_visible() {
            visitor(self.child.as_ref());
        }
    }
}

impl Rebuildable for AnimatedCollapseElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.child.rebuild_if_dirty(ctx);
    }
}

impl LayoutElement for AnimatedCollapseElement {
    fn event_tree_bounds(&self) -> Option<(Vec2d, Vec2d)> {
        None
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        let natural = self.natural_size(ctx);
        ResolvedSize {
            width: nonnegative_extent(natural.width),
            height: collapsed_height(natural.height, self.progress()),
        }
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.computed_size(ctx)
    }

    fn get_size_from_child(&self) -> Option<aimer_widget::base::Size> {
        self.child.get_size_from_child()
    }

    fn is_layout_stable(&self) -> bool {
        false
    }

    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.bounds.pos_start_end()
    }
}

#[inline]
fn list_edge_padding() -> LayoutSpacing {
    LayoutSpacing::all(8)
}

/// The always-visible header slot of a [`CollapsibleList`].
pub struct ListHeader<W = RequiredChild> {
    padding: LayoutSpacing,
    child: W,
}

impl ListHeader {
    /// Creates an empty header builder.
    #[inline]
    pub fn new() -> Self {
        Self {
            padding: LayoutSpacing::default(),
            child: RequiredChild,
        }
    }
}

impl<W> ListHeader<W> {
    /// Sets spacing inside the header slot.
    #[inline]
    pub fn padding(mut self, padding: LayoutSpacing) -> Self {
        self.padding = padding;
        self
    }

    /// Attaches the header child and completes the slot.
    #[inline]
    pub fn child<C: Widget + 'static>(self, child: C) -> ListHeader<C> {
        ListHeader {
            padding: self.padding,
            child,
        }
    }

    /// Attaches the header child and erases the completed slot type.
    #[inline]
    pub fn box_child<C: Widget + 'static>(self, child: C) -> AnyWidget {
        self.child(child).boxed()
    }
}

impl<W: Widget> Widget for ListHeader<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        Container::new()
            .padding(self.padding)
            .child(self.child)
            .to_element(ctx)
    }

    fn debug_name(&self) -> &'static str {
        "ListHeader"
    }
}

impl<W: Widget> PortableWidget for ListHeader<W> {}

/// The body slot of a [`CollapsibleList`].
///
/// The body is mounted lazily on the first expansion. After mounting, its
/// retained child remains owned by the list state while collapsed, allowing
/// the vertical reveal animation to finish without recreating the subtree.
pub struct ListBody<W = RequiredChild> {
    padding: LayoutSpacing,
    child: W,
}

impl ListBody {
    /// Creates an empty body builder.
    #[inline]
    pub fn new() -> Self {
        Self {
            padding: LayoutSpacing::default(),
            child: RequiredChild,
        }
    }
}

impl<W> ListBody<W> {
    /// Sets spacing inside the body slot.
    #[inline]
    pub fn padding(mut self, padding: LayoutSpacing) -> Self {
        self.padding = padding;
        self
    }

    /// Attaches the body child and completes the slot.
    #[inline]
    pub fn child<C: Widget + 'static>(self, child: C) -> ListBody<C> {
        ListBody {
            padding: self.padding,
            child,
        }
    }

    /// Attaches the body child and erases the completed slot type.
    #[inline]
    pub fn box_child<C: Widget + 'static>(self, child: C) -> AnyWidget {
        self.child(child).boxed()
    }
}

impl<W: Widget> Widget for ListBody<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        Container::new()
            .padding(self.padding)
            .child(self.child)
            .to_element(ctx)
    }

    fn debug_name(&self) -> &'static str {
        "ListBody"
    }
}

impl<W: Widget> PortableWidget for ListBody<W> {}

/// A stateful list whose header remains visible while its body can be toggled
/// with an animated vertical reveal.
///
/// The `expanded` builder value is read only when the list's state is first
/// created. Later pointer or keyboard activation changes the live state, and a
/// parent rebuild does not reset that state. The header and body are retained
/// through [`ChildBuilder`] so their widget subtrees survive those rebuilds.
/// Use [`Self::animation_duration`], [`Self::animation_millis`], and
/// [`Self::animation_curve`] to customize the timing of both transitions.
///
/// ```
/// use std::time::Duration;
///
/// use aimer_animation::curves;
/// use aimer_collapsible::{CollapsibleList, ListBody, ListHeader};
/// use aimer_text::Text;
///
/// let _list = CollapsibleList::new()
///     .animation_duration(Duration::from_millis(240))
///     .animation_curve(curves::EASE_OUT)
///     .header(ListHeader::new().child(Text::new("Tags")))
///     .body(ListBody::new().child(Text::new("Red")));
/// ```
pub struct CollapsibleList<H = RequiredChild, B = RequiredChild> {
    expanded: bool,
    duration: Duration,
    curve: AnyCurve,
    header: H,
    body: B,
}

impl CollapsibleList {
    /// Creates an empty list builder with an initially expanded body.
    #[inline]
    pub fn new() -> Self {
        Self {
            expanded: true,
            duration: Duration::from_millis(COLLAPSIBLE_ANIMATION_MILLIS),
            curve: curves::EASE_IN_OUT.into_any(),
            header: RequiredChild,
            body: RequiredChild,
        }
    }
}

impl<H, B> CollapsibleList<H, B> {
    /// Sets the initial expanded state.
    ///
    /// This value is used when the list first creates its state. Once mounted,
    /// user interaction owns the live expanded state until the widget is
    /// replaced by a different state identity.
    #[inline]
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }

    /// Sets the duration used by both the body reveal and chevron rotation.
    ///
    /// A zero duration makes the transition settle on its next animation
    /// tick without an intermediate animated interval.
    #[inline]
    pub fn animation_duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    /// Sets the duration used by both transitions in milliseconds.
    #[inline]
    pub fn animation_millis(self, millis: u64) -> Self {
        self.animation_duration(Duration::from_millis(millis))
    }

    /// Sets the easing curve used by both the body reveal and chevron rotation.
    #[inline]
    pub fn animation_curve(mut self, curve: impl Curve) -> Self {
        self.curve = curve.into_any();
        self
    }

    /// Attaches the always-visible header slot.
    #[inline]
    pub fn header<C: Widget + 'static>(
        self,
        header: ListHeader<C>,
    ) -> CollapsibleList<ListHeader<C>, B> {
        CollapsibleList {
            expanded: self.expanded,
            duration: self.duration,
            curve: self.curve,
            header,
            body: self.body,
        }
    }

    /// Attaches the collapsible body slot.
    #[inline]
    pub fn body<C: Widget + 'static>(self, body: ListBody<C>) -> CollapsibleList<H, ListBody<C>> {
        CollapsibleList {
            expanded: self.expanded,
            duration: self.duration,
            curve: self.curve,
            header: self.header,
            body,
        }
    }
}

/// Runtime state retained by [`CollapsibleList`].
pub struct CollapsibleListState<H: Widget + 'static, B: Widget + 'static> {
    expanded: bool,
    body_mounted: bool,
    duration: Duration,
    curve: AnyCurve,
    chevron_controller: AnimationController,
    body_controller: AnimationController,
    header: ChildBuilder,
    body: ChildBuilder,
    updater: StateUpdater<Self>,
    marker: PhantomData<(H, B)>,
}

impl<H: Widget + 'static, B: Widget + 'static> CollapsibleListState<H, B> {
    /// Reports whether the body is currently targeted as expanded.
    #[inline]
    pub fn is_expanded(&self) -> bool {
        self.expanded
    }

    #[inline]
    pub(crate) fn toggle(&mut self) {
        self.expanded = !self.expanded;
        if self.expanded {
            self.body_mounted = true;
            self.body_controller.forward();
            self.chevron_controller.reverse();
        } else {
            self.body_controller.reverse();
            self.chevron_controller.forward();
        }
    }

    #[cfg(test)]
    pub(crate) fn chevron_progress(&self) -> f32 {
        self.chevron_controller.value()
    }

    #[cfg(test)]
    pub(crate) fn body_progress(&self) -> f32 {
        self.body_controller.value()
    }

    #[cfg(test)]
    pub(crate) fn chevron_duration(&self) -> Duration {
        self.chevron_controller.duration()
    }

    #[cfg(test)]
    pub(crate) fn body_duration(&self) -> Duration {
        self.body_controller.duration()
    }

    #[cfg(test)]
    pub(crate) fn chevron_curve(&self) -> AnyCurve {
        self.chevron_controller.curve()
    }

    #[cfg(test)]
    pub(crate) fn body_curve(&self) -> AnyCurve {
        self.body_controller.curve()
    }
}

impl<H: Widget + 'static, B: Widget + 'static> StatefulWidget for CollapsibleList<H, B> {
    type State = CollapsibleListState<H, B>;

    fn create_state(self) -> Self::State {
        let chevron_controller = AnimationController::new(self.duration, self.curve.clone());
        if !self.expanded {
            chevron_controller.set_value(1.0);
        }
        let body_controller = AnimationController::new(self.duration, self.curve.clone());
        if self.expanded {
            body_controller.set_value(1.0);
        }

        CollapsibleListState {
            expanded: self.expanded,
            body_mounted: self.expanded,
            duration: self.duration,
            curve: self.curve,
            chevron_controller,
            body_controller,
            header: ChildBuilder::from_widget(self.header),
            body: ChildBuilder::from_widget(self.body),
            updater: StateUpdater::empty(),
            marker: PhantomData,
        }
    }
}

impl<H: Widget + 'static, B: Widget + 'static> Widget for CollapsibleList<H, B> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "CollapsibleList", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "CollapsibleList"
    }
}

impl<H: Widget + 'static, B: Widget + 'static> PortableWidget for CollapsibleList<H, B> {}

impl<H: Widget + 'static, B: Widget + 'static> State<CollapsibleList<H, B>>
    for CollapsibleListState<H, B>
{
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn adopt_config_from(&mut self, new: Self) {
        // `expanded` is live interaction state. The freshly built widget only
        // contributes its updated configuration and child widgets.
        self.duration = new.duration;
        self.curve = new.curve;
        self.chevron_controller.set_duration(self.duration);
        self.chevron_controller.set_curve(self.curve.clone());
        self.body_controller.set_duration(self.duration);
        self.body_controller.set_curve(self.curve.clone());
        self.header = new.header;
        self.body = new.body;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let theme = ctx.try_copied().unwrap_or(ThemeData::default());
        let updater = self.updater;
        let header = Container::new()
            .padding(list_edge_padding())
            .child(
                Row::new()
                    .vertical_alignment(BoxAlignment::Center)
                    .children([
                        Expanded::new().child(self.header.clone()).boxed(),
                        chevron(theme, self.chevron_controller.clone()),
                    ]),
            )
            .boxed();
        // The toggle owns the whole header hit region, so an interactive
        // descendant cannot intercept its pointer or focus events.
        let header = GestureDetector::new()
            .behavior(GestureDetectorBehavior::BlockChild)
            .on_tap(move || updater.set_state(|state| state.toggle()))
            .child(header);
        let header = KeyRelay::new()
            .on_activate(move || {
                updater.set_state(|state| state.toggle());
                true
            })
            .child(header);
        let header = aimer_widget::Focusable::new().child(header).boxed();

        let mut children = Vec::with_capacity(if self.body_mounted { 2 } else { 1 });
        children.push(header);
        if self.body_mounted {
            children.push(
                AnimatedCollapse::new(
                    self.body_controller.clone(),
                    Container::new()
                        .padding(list_edge_padding())
                        .child(self.body.clone()),
                )
                .boxed(),
            );
        }
        Column::new().children(children)
    }
}
