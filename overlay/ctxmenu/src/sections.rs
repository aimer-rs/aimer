//! A menu that has rows of widgets among its rows of words.
//!
//! The rows of words are one element that marks the row under the pointer and chooses it, and a widget takes its own events: they are not
//! one element's children. So a menu with widgets is built as a column of parts: a run of rows of words, a widget, another run, and a line
//! where a row asked to be set apart. Each run is the same rows element a menu of words has, and each widget is an ordinary child of the
//! column, so what is done on it is what is done on it anywhere.
//!
//! What is here is the plan, which is arithmetic and is asserted directly; building the parts from it is the menu's.

use std::rc::Rc;

use aimer_container::{Container, SizedBox};
use aimer_flex::{BoxAlignment, Column};
use aimer_style::LayoutSpacing;
use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, AnyWidget, Widget};

use crate::dismiss::ContextMenuDismiss;
use crate::item::ContextMenuItem;
use crate::rows::{ContextMenuRows, content_widths, geometry, icon_lead};
use crate::shape::ContextMenuShape;
use crate::style::ContextMenuStyle;

/// One part of the column.
pub(crate) enum Part {
    /// A run of rows of words, and the number of the first among all the menu's items: a choice is told by the number it has in the menu.
    Rows { base: usize, items: Vec<ContextMenuItem> },
    /// A row that is a widget, and the number it has.
    Widget { index: usize, item: ContextMenuItem },
    /// The line that sets what is below it apart from what is above.
    Divider,
}

/// Whether any item of `items` is a widget: a menu of words is built as it always was.
pub(crate) fn has_widgets(items: &[ContextMenuItem]) -> bool {
    items.iter().any(ContextMenuItem::is_widget)
}

/// The parts of the column for `items`, in order. Consecutive rows of words are one run, and a widget ends it. A line comes before a widget
/// that was set apart, and before a run that begins with a row that was, where there is something above them: a line inside a run is the
/// rows element's own.
pub(crate) fn plan(items: &[ContextMenuItem]) -> Vec<Part> {
    let mut parts: Vec<Part> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        if item.is_widget() {
            if item.has_separator() && !parts.is_empty() {
                parts.push(Part::Divider);
            }
            parts.push(Part::Widget { index, item: item.clone() });
            continue;
        }
        match parts.last_mut() {
            Some(Part::Rows { items, .. }) => items.push(item.clone()),
            _ => {
                if item.has_separator() && !parts.is_empty() {
                    parts.push(Part::Divider);
                }
                parts.push(Part::Rows { base: index, items: vec![item.clone()] });
            }
        }
    }
    parts
}

/// How wide every part of the column is, in logical pixels: as wide as the widest row of words needs, and not less than the floor of the
/// style or than any widget asks for. The parts are all this wide so that the rows, the widgets and the lines make one panel.
pub(crate) fn menu_width(style: &ContextMenuStyle, content_widths: &[f32], items: &[ContextMenuItem]) -> f32 {
    let words = geometry::item_widths(ContextMenuShape::List, style, content_widths).first().copied().unwrap_or(style.min_width);
    items.iter().map(ContextMenuItem::widget_min_width).fold(words, f32::max)
}

/// The space above the line, the line, and the space below it, which together are the `gap` that a line adds, in logical pixels.
pub(crate) fn divider_parts(gap: f32) -> (f32, f32, f32) {
    let side = ((gap - 1.0) * 0.5).max(0.0);
    (side, 1.0, side)
}

/// What the column is built from.
pub(crate) struct Sections<'a> {
    pub items: &'a [ContextMenuItem],
    pub style: &'a ContextMenuStyle,
    pub dismiss: &'a ContextMenuDismiss,
    pub dismiss_on_select: bool,
    /// What is told which row was chosen, by its number among all the menu's items.
    pub on_select: Option<&'a Rc<dyn Fn(usize)>>,
}

impl Sections<'_> {
    /// The column: runs of rows of words, the widgets, and the lines, all as wide as the widest of them needs. Measured here, at the scale the
    /// menu is drawn at, so that the runs agree on one width that no one of them could know alone.
    pub(crate) fn element(&self, ctx: &BuildContext) -> AnyElement {
        let lead = icon_lead(self.style, true, self.items, false);
        let measured = content_widths(ctx, self.items, self.style, true, lead);
        let words: Vec<f32> = measured.iter().zip(self.items).filter(|(_, item)| !item.is_widget()).map(|(width, _)| *width).collect();
        let width = menu_width(self.style, &words, self.items);
        // A run with no icon of its own keeps the room where another has one, so that the labels are in line.
        let icons = lead > 0.0;
        let parts: Vec<AnyWidget> = plan(self.items)
            .into_iter()
            .filter_map(|part| match part {
                Part::Rows { base, items } => {
                    let mut rows = ContextMenuRows::new()
                        .shape(ContextMenuShape::List)
                        .style(self.style.clone().min_width(width))
                        .items(items)
                        .dismiss_with(self.dismiss.clone())
                        .dismiss_on_select(self.dismiss_on_select)
                        .icon_column(icons);
                    if let Some(on_select) = self.on_select {
                        let on_select = Rc::clone(on_select);
                        rows = rows.on_select(move |index| on_select(base + index));
                    }
                    Some(rows.boxed())
                }
                Part::Widget { item, .. } => {
                    let height = item.widget_height()?;
                    let widget = item.build_widget(self.dismiss.clone())?;
                    Some(SizedBox::new().width(width).height(height).box_child(widget).boxed())
                }
                Part::Divider => Some(divider(self.style, width)),
            })
            .collect();
        Column::new().horizontal_alignment(BoxAlignment::Start).children(parts).to_element(ctx)
    }
}

/// The line between two groups: `style.separator_gap` tall, with the hairline in the middle of it and kept in from the sides as the labels are.
fn divider(style: &ContextMenuStyle, width: f32) -> AnyWidget {
    let (above, _, _) = divider_parts(style.separator_gap);
    let inset = style.item_padding as u32;
    let across = (width - style.item_padding * 2.0).max(0.0);
    // As the rules of the app are made: a box of the line's size, with a colour over a child of that size.
    let line = SizedBox::new()
        .width(across)
        .height(1.0)
        .box_child(Container::new().color(style.separator_color).child(SizedBox::new().width(across).height(1.0)));
    Container::new()
        .width(width)
        .height(style.separator_gap)
        .padding(LayoutSpacing::new().top(above as u32).left(inset))
        .child(line)
        .boxed()
}

#[cfg(test)]
mod tests {
    use aimer_container::ZeroSizedBox;

    use super::*;

    fn word(label: &'static str) -> ContextMenuItem {
        ContextMenuItem::new(label)
    }

    fn widget() -> ContextMenuItem {
        ContextMenuItem::widget(30.0, |_| ZeroSizedBox)
    }

    /// The plan as a word: `R2@0` is a run of two rows from the first item, `W@2` a widget that is the third, `-` a line.
    fn shape(parts: &[Part]) -> Vec<String> {
        parts
            .iter()
            .map(|part| match part {
                Part::Rows { base, items } => format!("R{}@{base}", items.len()),
                Part::Widget { index, .. } => format!("W@{index}"),
                Part::Divider => "-".to_owned(),
            })
            .collect()
    }

    #[test]
    fn a_menu_of_words_has_no_widgets_and_is_built_as_it_was() {
        assert!(!has_widgets(&[word("Copy"), word("Paste")]));
        assert!(!has_widgets(&[]));
        assert!(has_widgets(&[word("Copy"), widget()]));
    }

    #[test]
    fn rows_of_words_are_one_run_and_a_widget_ends_it_with_the_numbers_the_menu_gives_them() {
        let items = [word("a"), word("b"), widget(), word("c"), word("d"), word("e")];
        assert_eq!(shape(&plan(&items)), ["R2@0", "W@2", "R3@3"]);
        assert_eq!(shape(&plan(&[widget(), widget()])), ["W@0", "W@1"], "two widgets are not a run");
        assert_eq!(shape(&plan(&[widget(), word("a")])), ["W@0", "R1@1"], "a widget first");
        assert!(plan(&[]).is_empty());
    }

    #[test]
    fn a_line_comes_before_a_widget_that_was_set_apart_and_before_a_run_that_begins_with_such_a_row() {
        let items = [word("a"), widget().separated(), word("b").separated(), word("c")];
        assert_eq!(shape(&plan(&items)), ["R1@0", "-", "W@1", "-", "R2@2"]);
    }

    #[test]
    fn a_row_set_apart_inside_a_run_is_not_a_line_of_the_plan_because_the_rows_draw_it() {
        let items = [word("a"), word("b").separated(), word("c")];
        assert_eq!(shape(&plan(&items)), ["R3@0"]);
    }

    #[test]
    fn nothing_above_is_nothing_to_be_set_apart_from() {
        assert_eq!(shape(&plan(&[widget().separated()])), ["W@0"]);
        assert_eq!(shape(&plan(&[word("a").separated(), word("b")])), ["R2@0"]);
    }

    #[test]
    fn what_is_not_set_apart_is_side_by_side_with_nothing_between() {
        assert_eq!(shape(&plan(&[word("a"), widget(), word("b")])), ["R1@0", "W@1", "R1@2"]);
    }

    #[test]
    fn the_menu_is_as_wide_as_its_widest_row_of_words_and_not_less_than_the_floor_or_a_widget_asks() {
        let style = ContextMenuStyle::list();
        let items = [word("a"), widget()];
        assert_eq!(menu_width(&style, &[40.0], &items), style.min_width, "the floor wins over a short label");
        assert_eq!(menu_width(&style, &[400.0], &items), 400.0 + style.item_padding * 2.0);
        let wide = [word("a"), widget().min_width(500.0)];
        assert_eq!(menu_width(&style, &[400.0], &wide), 500.0, "a widget that cannot be narrower makes the menu wider");
        let only_widgets = [widget().min_width(10.0)];
        assert_eq!(menu_width(&style, &[], &only_widgets), style.min_width, "no words: the floor");
    }

    #[test]
    fn a_line_is_one_pixel_with_the_same_room_above_and_below_and_fills_the_gap_it_adds() {
        let (above, line, below) = divider_parts(9.0);
        assert_eq!((above, line, below), (4.0, 1.0, 4.0));
        assert_eq!(above + line + below, 9.0);
        assert_eq!(divider_parts(0.0), (0.0, 1.0, 0.0), "no gap leaves the line alone, and does not go negative");
    }

    fn headless_context() -> BuildContext<'static> {
        let canvas = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        BuildContext::new(
            aimer_canvas::FrameCanvas::new(canvas),
            aimer_attribute::ResolvedSize { width: 600.0, height: 600.0 },
            1.0,
            Default::default(),
            Default::default(),
            aimer_widget::base::WindowHandle::headless(Default::default(), 1.0),
            #[cfg(not(target_arch = "wasm32"))]
            tokio::runtime::Handle::current(),
        )
    }

    /// The size that the column of `items` takes laid out in a window big enough for it.
    fn laid_out(items: Vec<ContextMenuItem>) -> (f32, f32) {
        let ctx = headless_context();
        let style = ContextMenuStyle::list();
        let dismiss = ContextMenuDismiss::new();
        let element = Sections { items: &items, style: &style, dismiss: &dismiss, dismiss_on_select: true, on_select: None }.element(&ctx);
        let size = element.layout(&ctx);
        (size.width, size.height)
    }

    #[tokio::test]
    async fn the_column_is_as_tall_as_its_rows_its_widgets_and_its_lines_and_as_wide_as_the_menu() {
        let style = ContextMenuStyle::list();
        let (width, height) = laid_out(vec![word("Copy"), widget().separated(), word("Paste")]);
        // Two rows of words, a widget of 30, and the line that sets the widget apart.
        assert_eq!(height, style.row_height * 2.0 + 30.0 + style.separator_gap, "the parts stack with nothing between them");
        assert_eq!(width, style.min_width, "short words: the floor of the style");
    }

    #[tokio::test]
    async fn a_widget_that_asks_for_a_width_makes_every_part_that_wide() {
        let (width, _) = laid_out(vec![word("Copy"), ContextMenuItem::widget(24.0, |_| ZeroSizedBox).min_width(320.0)]);
        assert_eq!(width, 320.0);
    }

    #[tokio::test]
    async fn a_menu_of_two_widgets_has_no_rows_and_is_as_tall_as_them() {
        let (_, height) = laid_out(vec![widget(), ContextMenuItem::widget(12.0, |_| ZeroSizedBox)]);
        assert_eq!(height, 42.0);
    }
}
