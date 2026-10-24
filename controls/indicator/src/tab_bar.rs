//! A tab bar with a sliding indicator.

use std::rc::Rc;
use std::time::Duration;

use aimer_animation::{AnyMotion, Motion, motions};
use aimer_attribute::{Dimension, ResolvedSize};
use aimer_container::{Container, SizedBox};
use aimer_decoration::{BoxDecoration, BoxShadow, Decoration};
use aimer_flex::{BoxAlignment, Column, JustifyContent, Row};
use aimer_space::{Align, Alignment};
use aimer_style::{FontWeight, LayoutSpacing, TextStyle};
use aimer_text::Text;
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{
    AnyElement, AnyWidget, PortableWidget, State, StateUpdater, StatefulElement, StatefulWidget,
    Widget,
};

mod chrome;

use chrome::BarChrome;

use crate::controller::IndicatorController;
use crate::sliding_indicator::SlidingIndicator;
use crate::style::{AnyIndicatorStyle, IndicatorStyle};
use crate::track::tracks;

/// The tint of the selected item.
const ACTIVE: Color = Color::Rgba(0, 122, 255, 255);
/// The tint of the other items.
const INACTIVE: Color = Color::Rgba(28, 28, 30, 255);
/// The bar behind the items.
const BAR_FILL: Color = Color::Rgba(250, 250, 252, 255);
const ICON_GAP: f32 = 3.0;
const LABEL_SIZE: u32 = 11;

type IconBuilder = Rc<dyn Fn(Color) -> AnyWidget>;
type SelectCallback = Rc<dyn Fn(usize)>;

/// One item of a [`TabBar`]: an icon over a label.
///
/// ```
/// use aimer_container::SizedBox;
/// use aimer_indicator::TabItem;
///
/// let item = TabItem::new()
///     .label("Recents")
///     .icon(|_tint| SizedBox::new().width(24.0).height(24.0));
/// ```
pub struct TabItem {
    label: Option<String>,
    icon: Option<IconBuilder>,
    selected_color: Option<Color>,
}

impl TabItem {
    /// Creates an item with no icon and no label.
    #[inline]
    pub fn new() -> Self {
        Self { label: None, icon: None, selected_color: None }
    }

    /// Sets the text under the icon.
    #[inline]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Sets the icon. The bar calls `icon` with the tint the item should be
    /// drawn in, which changes when the item is selected, and builds the icon
    /// again whenever that happens.
    #[inline]
    pub fn icon<W: Widget + 'static>(mut self, icon: impl Fn(Color) -> W + 'static) -> Self {
        self.icon = Some(Rc::new(move |tint| icon(tint).boxed()));
        self
    }

    /// Sets the tint used while this item is selected. The default is the
    /// system blue.
    #[inline]
    pub fn selected_color(mut self, color: Color) -> Self {
        self.selected_color = Some(color);
        self
    }
}

impl Default for TabItem {
    fn default() -> Self {
        Self::new()
    }
}

/// A row of items with an indicator that slides to the selected one.
///
/// The bar is *controlled*: it shows the item at [`selected`](Self::selected),
/// and tapping another one only proposes it through
/// [`on_select`](Self::on_select); the bar moves once the caller passes the new
/// index back in. While a pointer is held on an item the indicator lifts, and
/// with the default [`LIQUID_GLASS`](crate::indicator_styles::LIQUID_GLASS) style
/// it becomes a lens, then settles again when the move is done.
///
/// The indicator can also be dragged. Pressing an item and moving the pointer
/// carries the indicator along, kept on the bar, and letting go proposes the
/// item it is nearest to; nothing is proposed while it is held.
///
/// Items share one size, [`item_size`](Self::item_size), so that the indicator's
/// layers can be built once and only moved.
///
/// ```
/// use aimer_container::SizedBox;
/// use aimer_indicator::{TabBar, TabItem, indicator_styles};
///
/// let bar = TabBar::new()
///     .selected(1)
///     .on_select(|index| println!("selected {index}"))
///     .style(indicator_styles::LIQUID_GLASS)
///     .item(TabItem::new().label("Recents").icon(|_| SizedBox::new().width(24.0).height(24.0)))
///     .item(TabItem::new().label("Shared").icon(|_| SizedBox::new().width(24.0).height(24.0)));
/// ```
pub struct TabBar {
    items: Vec<TabItem>,
    selection: Option<usize>,
    bar_color: Color,
    inactive_color: Color,
    selected_color: Color,
    bar_shadow: Option<BoxShadow>,
    chrome: BarChrome,
    on_select: Option<SelectCallback>,
    item_size: ResolvedSize,
    style: AnyIndicatorStyle,
    motion: AnyMotion,
    lift_motion: AnyMotion,
    reduced_motion: bool,
    controller: Option<IndicatorController>,
}

impl TabBar {
    /// Creates an empty bar with 96 by 56 items, the liquid glass indicator, and
    /// a bouncy spring. Pressing an item lifts the indicator, and it can be
    /// dragged along the bar.
    #[inline]
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            selection: Some(0),
            bar_color: BAR_FILL,
            inactive_color: INACTIVE,
            selected_color: ACTIVE,
            bar_shadow: None,
            chrome: BarChrome::new(),
            on_select: None,
            item_size: ResolvedSize { width: 96.0, height: 56.0 },
            style: AnyIndicatorStyle::default(),
            motion: motions::BOUNCY.into_any(),
            lift_motion: motions::SNAPPY.into_any(),
            reduced_motion: false,
            controller: None,
        }
    }

    /// Appends an item.
    #[inline]
    pub fn item(mut self, item: TabItem) -> Self {
        self.items.push(item);
        self
    }

    /// Appends every item of `items`, for a bar built from data.
    #[inline]
    pub fn items(mut self, items: impl IntoIterator<Item = TabItem>) -> Self {
        self.items.extend(items);
        self
    }

    /// Sets which item is selected. An index past the last item selects the
    /// last one.
    #[inline]
    pub fn selected(mut self, selected: usize) -> Self {
        self.selection = Some(selected);
        self
    }

    /// Sets which item is selected, or `None` for a bar with nothing selected,
    /// such as one shown on a page that none of its items leads to.
    ///
    /// With nothing selected no item is tinted and the indicator fades out
    /// where it was. Pressing any item brings it back under the pointer, and
    /// letting go proposes the item it is on, since none was selected to begin
    /// with.
    #[inline]
    pub fn selection(mut self, selection: impl Into<Option<usize>>) -> Self {
        self.selection = selection.into();
        self
    }

    /// Sets the color of the bar behind the items. The default is a near-white
    /// that suits a light page; a dark theme wants a dark one.
    #[inline]
    pub fn bar_color(mut self, color: Color) -> Self {
        self.bar_color = color;
        self
    }

    /// Sets the tint of the items that are not selected, for their icons and
    /// labels.
    #[inline]
    pub fn inactive_color(mut self, color: Color) -> Self {
        self.inactive_color = color;
        self
    }

    /// Sets the tint of the selected item. An item's own
    /// [`selected_color`](TabItem::selected_color) takes precedence. The default
    /// is the system blue.
    #[inline]
    pub fn selected_color(mut self, color: Color) -> Self {
        self.selected_color = color;
        self
    }

    /// Gives the bar a shadow, to lift it off a page it floats over.
    #[inline]
    pub fn bar_shadow(mut self, shadow: BoxShadow) -> Self {
        self.bar_shadow = Some(shadow);
        self
    }

    /// Replaces what the bar's container is painted with.
    ///
    /// The default is a pill filled with [`bar_color`](Self::bar_color) and
    /// casting [`bar_shadow`](Self::bar_shadow); those two only shape that
    /// default, and a decoration set here is used as it is. Its content insets,
    /// such as a border's width, are added to the bar's size so the items keep
    /// their room, and it decides whether the items are clipped to its shape
    /// (see [`clip_content`](Self::clip_content)).
    ///
    /// ```
    /// use aimer_decoration::BoxDecoration;
    /// use aimer_indicator::TabBar;
    /// use aimer_widget::base::Color;
    ///
    /// let bar = TabBar::new()
    ///     .bar_decoration(BoxDecoration::new().background_color(Color::BLUE).border_radius(12));
    /// ```
    #[inline]
    pub fn bar_decoration(mut self, decoration: impl Decoration + 'static) -> Self {
        self.chrome.decoration = Some(Rc::new(decoration));
        self
    }

    /// Sets the space between the bar's edge and its items. The default is six
    /// pixels on every side.
    ///
    /// The bar grows or shrinks to fit, and the indicator follows the items.
    /// Only pixel values add to the bar's size; a percentage has no parent to
    /// resolve against and counts as nothing.
    #[inline]
    pub fn bar_padding(mut self, padding: LayoutSpacing) -> Self {
        self.chrome.padding = padding;
        self
    }

    /// Sets the transparent space outside the bar. The default is none.
    #[inline]
    pub fn bar_margin(mut self, margin: LayoutSpacing) -> Self {
        self.chrome.margin = margin;
        self
    }

    /// Sets the bar's outer width, margin excluded.
    ///
    /// By default, and for [`Dimension::Auto`], the bar is as wide as its items
    /// and padding. A larger width leaves the extra space after the last item.
    /// A percentage is resolved by the container against its parent.
    #[inline]
    pub fn bar_width(mut self, width: impl Into<Dimension>) -> Self {
        self.chrome.width = width.into();
        self
    }

    /// Sets the bar's outer height, margin excluded. See
    /// [`bar_width`](Self::bar_width).
    #[inline]
    pub fn bar_height(mut self, height: impl Into<Dimension>) -> Self {
        self.chrome.height = height.into();
        self
    }

    /// Places the bar within the space its parent gives it. By default the bar
    /// is left where the parent puts it.
    #[inline]
    pub fn bar_alignment(mut self, alignment: Alignment) -> Self {
        self.chrome.alignment = Some(alignment);
        self
    }

    /// Sets whether the items and the indicator are clipped to the shape the
    /// decoration gives, which is the default. With `false` the indicator can be
    /// lifted past the bar's edge, such as a lens that grows when pressed.
    #[inline]
    pub fn clip_content(mut self, clip: bool) -> Self {
        self.chrome.clip_content = clip;
        self
    }

    /// Sets the callback that receives the index of an item the user tapped.
    #[inline]
    pub fn on_select(mut self, on_select: impl Fn(usize) + 'static) -> Self {
        self.on_select = Some(Rc::new(on_select));
        self
    }

    /// Sets the size of every item, in logical pixels. Values that are not
    /// finite and positive keep the current size.
    #[inline]
    pub fn item_size(mut self, width: f32, height: f32) -> Self {
        if width.is_finite() && width > 0.0 && height.is_finite() && height > 0.0 {
            self.item_size = ResolvedSize { width, height };
        }
        self
    }

    /// Sets how the indicator looks.
    #[inline]
    pub fn style(mut self, style: impl IndicatorStyle) -> Self {
        self.style = style.into_any();
        self
    }

    /// Sets how the indicator travels between items.
    #[inline]
    pub fn motion(mut self, motion: impl Motion) -> Self {
        self.motion = motion.into_any();
        self
    }

    /// Sets how the indicator lifts and settles.
    #[inline]
    pub fn lift_motion(mut self, motion: impl Motion) -> Self {
        self.lift_motion = motion.into_any();
        self
    }

    /// Makes the indicator jump and never lift, for a user who asked the system
    /// to reduce motion.
    #[inline]
    pub fn reduced_motion(mut self, reduced: bool) -> Self {
        self.reduced_motion = reduced;
        self
    }

    /// Sets how long the indicator takes to lift into its lifted form and to
    /// settle back, replacing the lift motion with a constant-speed ramp of
    /// that length. See [`IndicatorController::fade_duration`].
    #[inline]
    pub fn fade_duration(mut self, duration: Duration) -> Self {
        self.lift_motion = motions::ramp(duration).into_any();
        self
    }

    /// Drives the indicator with `controller` instead of one the bar makes for
    /// itself, so that the caller can see where the indicator is, how lifted it
    /// is, and whether it is being dragged.
    ///
    /// The controller keeps its own motions: [`motion`](Self::motion),
    /// [`lift_motion`](Self::lift_motion) and
    /// [`reduced_motion`](Self::reduced_motion) only configure a controller the
    /// bar makes. The bar still moves it to follow [`selected`](Self::selected).
    #[inline]
    pub fn controller(mut self, controller: IndicatorController) -> Self {
        self.controller = Some(controller);
        self
    }

    /// The selection limited to the items that exist.
    fn clamped_selection(&self) -> Option<usize> {
        let last = self.items.len().checked_sub(1)?;
        self.selection.map(|selected| selected.min(last))
    }
}

impl Default for TabBar {
    fn default() -> Self {
        Self::new()
    }
}

/// Retained state for a [`TabBar`]: the indicator's motion, which has to
/// outlive the rebuild that a selection change causes.
pub struct TabBarState {
    items: Vec<TabItem>,
    selection: Option<usize>,
    bar_color: Color,
    inactive_color: Color,
    selected_color: Color,
    bar_shadow: Option<BoxShadow>,
    chrome: BarChrome,
    on_select: Option<SelectCallback>,
    item_size: ResolvedSize,
    style: AnyIndicatorStyle,
    controller: IndicatorController,
}

impl TabBarState {
    /// Where the items are: one slot each, left to right.
    fn track(&self) -> crate::track::Linear {
        tracks::row(self.items.len(), self.item_size.width, self.item_size.height)
    }

    /// The indicator, listening to the pointer if the bar has anywhere to send
    /// what the user chose.
    fn indicator(&self) -> SlidingIndicator {
        let indicator = SlidingIndicator::new();
        match &self.on_select {
            Some(on_select) => {
                let on_select = on_select.clone();
                indicator.on_select(move |index| on_select(index))
            }
            None => indicator,
        }
    }

    /// The bar's outer size at `scale`, margin excluded.
    fn bar_extent(&self, scale: f32) -> ResolvedSize {
        let content = ResolvedSize {
            width: self.item_size.width * self.items.len() as f32,
            height: self.item_size.height,
        };
        self.chrome.extent(content, scale)
    }

    /// The tint of item `index`: the selected color if it is the selected item,
    /// the inactive one otherwise.
    fn tint(&self, index: usize, item: &TabItem) -> Color {
        if self.selection == Some(index) {
            item.selected_color.unwrap_or(self.selected_color)
        } else {
            self.inactive_color
        }
    }

    /// One item: its icon over its label, in the size of a slot.
    fn item(&self, index: usize, item: &TabItem) -> AnyWidget {
        let tint = self.tint(index, item);
        let mut content: Vec<AnyWidget> = Vec::with_capacity(3);
        if let Some(icon) = &item.icon {
            content.push(icon(tint));
        }
        if let Some(label) = &item.label {
            if !content.is_empty() {
                content.push(SizedBox::new().height(ICON_GAP).boxed());
            }
            content.push(
                Text::new(label.clone())
                    .text_style(
                        TextStyle::new()
                            .font_size(LABEL_SIZE)
                            .font_weight(FontWeight::Value(600))
                            .color(tint),
                    )
                    .boxed(),
            );
        }

        Container::new()
            .width(self.item_size.width)
            .height(self.item_size.height)
            .box_child(
                Column::new()
                    .justify_content(JustifyContent::Center)
                    .horizontal_alignment(BoxAlignment::Center)
                    .children(content),
            )
    }
}

impl StatefulWidget for TabBar {
    type State = TabBarState;

    fn create_state(self) -> Self::State {
        let selection = self.clamped_selection();
        let controller = self.controller.unwrap_or_else(|| {
            IndicatorController::new(selection.unwrap_or(0) as f32)
                .motion(self.motion)
                .lift_motion(self.lift_motion)
                .reduced_motion(self.reduced_motion)
        });
        if selection.is_none() {
            controller.set_hidden(true);
        }
        TabBarState {
            items: self.items,
            selection,
            bar_color: self.bar_color,
            inactive_color: self.inactive_color,
            selected_color: self.selected_color,
            bar_shadow: self.bar_shadow,
            chrome: self.chrome,
            on_select: self.on_select,
            item_size: self.item_size,
            style: self.style,
            controller,
        }
    }
}

impl State<TabBar> for TabBarState {
    fn init_state(&mut self, _updater: StateUpdater<Self>) {}

    fn adopt_config_from(&mut self, new: Self) {
        self.items = new.items;
        self.on_select = new.on_select;
        self.item_size = new.item_size;
        self.style = new.style;
        self.selection = new.selection;
        self.bar_color = new.bar_color;
        self.inactive_color = new.inactive_color;
        self.selected_color = new.selected_color;
        self.bar_shadow = new.bar_shadow;
        self.chrome = new.chrome;
        // Always bring the indicator back to the selection: a drag the caller
        // did not accept leaves it somewhere else. A drag in progress is not
        // interrupted.
        match self.selection {
            Some(selected) => {
                if self.controller.is_hidden() {
                    // Coming back from nothing selected: fade in on the item, do
                    // not slide in from wherever it was left.
                    self.controller.jump_to(selected as f32);
                    self.controller.set_hidden(false);
                } else {
                    self.controller.select(selected as f32);
                }
            }
            None => self.controller.set_hidden(true),
        }
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let items: Vec<AnyWidget> = self
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| self.item(index, item))
            .collect();
        let size = self.bar_extent(ctx.scale);
        let content = self
            .indicator()
            .controller(self.controller.clone())
            .track(self.track())
            .style(self.style.clone())
            .box_child(Row::new().children(items));

        let bar = match &self.chrome.decoration {
            Some(decoration) => self.chrome.frame(decoration.clone(), size, content),
            None => {
                let mut decoration = BoxDecoration::new()
                    .background_color(self.bar_color)
                    .border_radius(size.height * 0.5);
                if let Some(shadow) = self.bar_shadow {
                    decoration = decoration.add_shadow(shadow);
                }
                self.chrome.frame(decoration, size, content)
            }
        };
        match self.chrome.alignment {
            Some(alignment) => Align::new().alignment(alignment).box_child(bar),
            None => bar,
        }
    }
}

impl Widget for TabBar {
    fn debug_name(&self) -> &'static str {
        "TabBar"
    }

    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "TabBar", None).0.boxed()
    }
}

impl PortableWidget for TabBar {}

#[cfg(test)]
mod tests {
    use aimer_animation::AnimInstant;

    use super::*;

    const RED: Color = Color::Rgba(255, 0, 0, 255);
    const GREEN: Color = Color::Rgba(0, 255, 0, 255);
    const BLUE: Color = Color::Rgba(0, 0, 255, 255);

    fn three() -> TabBar {
        TabBar::new().item(TabItem::new()).item(TabItem::new()).item(TabItem::new())
    }

    #[test]
    fn the_fade_duration_sets_the_lift_of_the_bars_own_controller() {
        let state = three().fade_duration(Duration::from_millis(100)).create_state();
        state.controller.set_pressed(true);
        let start = AnimInstant::now();
        state.controller.tick(start);
        let lift = state.controller.tick(start + Duration::from_millis(60)).lift;
        // A 100 ms ramp is 60% of the way after 60 ms, where the default spring
        // is not yet at half.
        assert!((lift - 0.6).abs() < 0.05, "lift {lift}");
    }

    #[test]
    fn items_appends_after_the_ones_already_added() {
        let bar = TabBar::new()
            .item(TabItem::new().label("a"))
            .items((0..3).map(|index| TabItem::new().label(format!("b{index}"))));
        let labels: Vec<_> = bar.items.iter().filter_map(|item| item.label.clone()).collect();
        assert_eq!(labels, ["a", "b0", "b1", "b2"]);
    }

    #[test]
    fn an_index_past_the_last_item_selects_the_last_and_an_empty_bar_selects_nothing() {
        assert_eq!(three().selected(9).clamped_selection(), Some(2));
        assert_eq!(three().selected(1).clamped_selection(), Some(1));
        assert_eq!(TabBar::new().selected(0).clamped_selection(), None);
    }

    #[test]
    fn a_bar_with_nothing_selected_starts_with_its_indicator_hidden() {
        let state = three().selection(None).create_state();
        assert_eq!(state.selection, None);
        assert!(state.controller.is_hidden());
        let shown = three().selected(1).create_state();
        assert!(!shown.controller.is_hidden());
        assert_eq!(shown.controller.target(), 1.0);
    }

    #[test]
    fn the_selected_item_is_tinted_and_the_rest_are_not() {
        let state = three()
            .selected(1)
            .selected_color(GREEN)
            .inactive_color(RED)
            .create_state();
        let item = TabItem::new();
        assert_eq!(state.tint(1, &item), GREEN);
        assert_eq!(state.tint(0, &item), RED);
        assert_eq!(state.tint(2, &item), RED);
    }

    #[test]
    fn an_items_own_selected_color_wins_over_the_bars() {
        let state = three().selected(0).selected_color(GREEN).create_state();
        assert_eq!(state.tint(0, &TabItem::new().selected_color(BLUE)), BLUE);
        assert_eq!(state.tint(0, &TabItem::new()), GREEN);
    }

    #[test]
    fn with_nothing_selected_no_item_is_tinted_as_selected() {
        let state = three().selection(None).selected_color(GREEN).inactive_color(RED).create_state();
        for index in 0..3 {
            assert_eq!(state.tint(index, &TabItem::new()), RED);
        }
    }

    #[test]
    fn the_default_colors_are_unchanged_when_none_are_set() {
        let state = three().selected(0).create_state();
        assert_eq!(state.tint(0, &TabItem::new()), ACTIVE);
        assert_eq!(state.tint(1, &TabItem::new()), INACTIVE);
        assert_eq!(state.bar_color, BAR_FILL);
        assert!(state.bar_shadow.is_none());
    }

    #[test]
    fn the_bars_colors_and_shadow_are_adopted_when_the_caller_changes_them() {
        let mut state = three().selected(0).create_state();
        let shadow = BoxShadow::new().blur(12.0);
        let next = three().selected(0).bar_color(BLUE).bar_shadow(shadow).create_state();
        state.adopt_config_from(next);
        assert_eq!(state.bar_color, BLUE);
        assert_eq!(state.bar_shadow, Some(shadow));
    }

    #[test]
    fn leaving_the_selection_hides_the_indicator_and_returning_fades_it_in_on_the_item() {
        let mut state = three().selected(0).create_state();
        state.adopt_config_from(three().selection(None).create_state());
        assert!(state.controller.is_hidden());
        assert_eq!(state.selection, None);

        state.adopt_config_from(three().selected(2).create_state());
        assert!(!state.controller.is_hidden());
        // It did not slide across from where it was left.
        assert_eq!(state.controller.position(), 2.0);
        assert_eq!(state.controller.target(), 2.0);
    }

    fn extent(bar: TabBar) -> (f32, f32) {
        let size = bar.create_state().bar_extent(1.0);
        (size.width, size.height)
    }

    /// A decoration that insets its child by `INSET` on every side.
    struct Inset;

    impl Decoration for Inset {
        fn paint(&self, _: &aimer_canvas::Canvas, _: aimer_attribute::Vec2d, _: ResolvedSize, _: f32) {}

        fn content_insets(&self, _: ResolvedSize, _: f32) -> [f32; 4] {
            [3.0; 4]
        }
    }

    #[test]
    fn the_default_bar_is_its_items_plus_six_pixels_of_padding() {
        assert_eq!(extent(three()), (96.0 * 3.0 + 12.0, 56.0 + 12.0));
    }

    #[test]
    fn bar_padding_grows_the_bar_around_its_items_side_by_side() {
        let padding = LayoutSpacing::new().left(4).right(10).top(2).bottom(8);
        assert_eq!(extent(three().bar_padding(padding)), (96.0 * 3.0 + 14.0, 56.0 + 10.0));
    }

    #[test]
    fn a_decorations_content_insets_grow_the_bar_too() {
        let bar = three().bar_padding(LayoutSpacing::all(6)).bar_decoration(Inset);
        assert_eq!(extent(bar), (96.0 * 3.0 + 12.0 + 6.0, 56.0 + 12.0 + 6.0));
    }

    #[test]
    fn an_explicit_bar_size_replaces_the_computed_one_per_axis() {
        assert_eq!(extent(three().bar_width(400)), (400.0, 56.0 + 12.0));
        assert_eq!(extent(three().bar_height(80)), (96.0 * 3.0 + 12.0, 80.0));
        // Auto and a percentage cannot be resolved here, so the computed size stands.
        assert_eq!(
            extent(three().bar_width(Dimension::Auto).bar_height(Dimension::Percent(50.0))),
            (96.0 * 3.0 + 12.0, 56.0 + 12.0)
        );
    }

    #[test]
    fn the_container_settings_are_adopted_when_the_caller_changes_them() {
        let mut state = three().create_state();
        assert!(state.chrome.decoration.is_none());
        let next = three()
            .bar_decoration(Inset)
            .bar_margin(LayoutSpacing::all(9))
            .bar_alignment(Alignment::BotRight)
            .clip_content(false)
            .create_state();
        state.adopt_config_from(next);
        assert!(state.chrome.decoration.is_some());
        assert_eq!(state.chrome.margin, LayoutSpacing::all(9));
        assert_eq!(state.chrome.alignment, Some(Alignment::BotRight));
        assert!(!state.chrome.clip_content);
    }

    #[test]
    fn a_new_selection_while_shown_slides_the_indicator() {
        let mut state = three().selected(0).create_state();
        state.adopt_config_from(three().selected(2).create_state());
        assert_eq!(state.controller.target(), 2.0);
        assert_eq!(state.controller.position(), 0.0);
    }
}
