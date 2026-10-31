//! Wrapping flex layout.
//!
//! [`OverflowBehavior::Wrap`] turns a single flex line into as many lines as the
//! constraints require: children fill the main axis until the next one no longer
//! fits, then continue on a fresh line offset along the cross axis. Positions
//! are resolved for the whole child list, so wrapping is intentionally eager —
//! a line break depends on every preceding child.

use std::rc::Rc;

use aimer_attribute::BoxConstraint;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_widget::Element;
use aimer_widget::base::BuildContext;

use crate::flex::raw_flex::{RawFlex, justify_distribution};
use crate::flex::{BoxAlignment, FlexDirection, JustifyContent};

/// Positions and total size of a wrapped flex line set.
pub(crate) struct WrapLayout {
    /// Top-left offset of every child, in child order.
    offsets: Vec<(f32, f32)>,
    /// Size occupied by all lines together, gaps included.
    pub(crate) size: ResolvedSize,
}

impl WrapLayout {
    /// Returns the top-left offset of a child in this wrapped layout.
    #[inline]
    pub(crate) fn offset(&self, index: usize) -> (f32, f32) {
        self.offsets[index]
    }
}

/// A wrapped layout remembered for one constraint and scale.
///
/// It lives in the container's [`LayoutCache`](aimer_widget::LayoutCache)
/// auxiliary slot and is only trusted while the cache's computed-size entry
/// for the same key is still valid, so it shares that entry's invalidation.
struct CachedWrap {
    constraint: BoxConstraint,
    scale_bits: u32,
    layout: Rc<(Vec<ResolvedSize>, WrapLayout)>,
}

/// Lays children out onto as many lines as `max_width` / `max_height` allow.
fn compute_wrap_layout(
    children: &[ResolvedSize],
    direction: FlexDirection,
    max_width: f32,
    max_height: f32,
    gap_x: f32,
    gap_y: f32,
    justify_content: JustifyContent,
) -> WrapLayout {
    let mut offsets = Vec::with_capacity(children.len());
    let mut lines = Vec::new();
    let mut line_start = 0;
    let mut main_offset: f32 = 0.0;
    let mut cross_offset: f32 = 0.0;
    let mut line_cross: f32 = 0.0;
    let mut max_line_main: f32 = 0.0;

    for child in children {
        let (child_main, child_cross, max_main, main_gap, cross_gap) = match direction {
            FlexDirection::Row | FlexDirection::Inherit => {
                (child.width, child.height, max_width, gap_x, gap_y)
            }
            FlexDirection::Column => (child.height, child.width, max_height, gap_y, gap_x),
        };
        let has_line_child = line_start < offsets.len();
        let required_main = if has_line_child {
            main_offset + main_gap + child_main
        } else {
            child_main
        };
        if has_line_child && required_main > max_main {
            lines.push((line_start, offsets.len(), main_offset));
            max_line_main = max_line_main.max(main_offset);
            cross_offset += line_cross + cross_gap;
            main_offset = 0.0;
            line_cross = 0.0;
            line_start = offsets.len();
        }
        if line_start < offsets.len() {
            main_offset += main_gap;
        }
        let offset = match direction {
            FlexDirection::Row | FlexDirection::Inherit => (main_offset, cross_offset),
            FlexDirection::Column => (cross_offset, main_offset),
        };
        offsets.push(offset);
        main_offset += child_main;
        line_cross = line_cross.max(child_cross);
    }

    if line_start < offsets.len() {
        lines.push((line_start, offsets.len(), main_offset));
    }
    max_line_main = max_line_main.max(main_offset);
    let total_cross = if children.is_empty() {
        0.0
    } else {
        cross_offset + line_cross
    };
    let size = match direction {
        FlexDirection::Row | FlexDirection::Inherit => ResolvedSize {
            width: max_line_main,
            height: total_cross,
        },
        FlexDirection::Column => ResolvedSize {
            width: total_cross,
            height: max_line_main,
        },
    };
    for (start, end, line_main) in lines {
        let (max_main, is_row) = match direction {
            FlexDirection::Row | FlexDirection::Inherit => (max_width, true),
            FlexDirection::Column => (max_height, false),
        };
        let (leading, between) = justify_distribution(
            justify_content,
            (max_main - line_main).max(0.0),
            end - start,
        );
        for (local_index, offset) in offsets[start..end].iter_mut().enumerate() {
            let extra = leading + between * local_index as f32;
            if is_row {
                offset.0 += extra;
            } else {
                offset.1 += extra;
            }
        }
    }

    WrapLayout { offsets, size }
}

impl RawFlex {
    pub(crate) fn wrapped_layout(
        &self,
        ctx: &BuildContext,
        gap_x: f32,
        gap_y: f32,
    ) -> (Vec<ResolvedSize>, WrapLayout) {
        // A line break depends on every preceding child, so wrapping cannot be
        // windowed: a data-driven source has to hand over its whole range.
        self.materialize_all(ctx);
        let mut child_ctx = ctx.clone();
        match self.direction {
            FlexDirection::Row | FlexDirection::Inherit => {
                child_ctx.box_constraint.max_width = f32::MAX;
            }
            FlexDirection::Column => {
                child_ctx.box_constraint.max_height = f32::MAX;
            }
        }
        let sizes = (0..self.children.len())
            .map(|index| match self.children.get(index) {
                Some(child) => child.computed_size(&child_ctx),
                None => ResolvedSize::default(),
            })
            .collect::<Vec<_>>();
        let layout = compute_wrap_layout(
            &sizes,
            self.direction,
            ctx.box_constraint.max_width,
            ctx.box_constraint.max_height,
            gap_x,
            gap_y,
            self.justify_content.unwrap_or(match self.direction {
                FlexDirection::Row | FlexDirection::Inherit => match self.horizontal_alignment {
                    BoxAlignment::Start => JustifyContent::Start,
                    BoxAlignment::Center => JustifyContent::Center,
                    BoxAlignment::End => JustifyContent::End,
                },
                FlexDirection::Column => match self.vertical_alignment {
                    BoxAlignment::Start => JustifyContent::Start,
                    BoxAlignment::Center => JustifyContent::Center,
                    BoxAlignment::End => JustifyContent::End,
                },
            }),
        );
        (sizes, layout)
    }

    /// Returns the wrapped layout for `ctx`, measuring only when the cached
    /// computed size has been invalidated or the constraint changed.
    ///
    /// The retained tree asks for every child's slot in turn, so recomputing
    /// the whole line set per child would be quadratic in the child count.
    pub(crate) fn wrapped_layout_cached(
        &self,
        ctx: &BuildContext,
    ) -> Rc<(Vec<ResolvedSize>, WrapLayout)> {
        let scale_bits = ctx.scale.to_bits();
        let constraint = ctx.box_constraint;
        let valid = self.cache.get_computed(constraint, scale_bits).is_some();
        if valid {
            let hit = self.cache.with_extra(|slot: &mut Option<CachedWrap>| {
                slot.as_ref()
                    .filter(|cached| {
                        cached.constraint == constraint && cached.scale_bits == scale_bits
                    })
                    .map(|cached| cached.layout.clone())
            });
            if let Some(hit) = hit {
                return hit;
            }
        }
        let (gap_x, gap_y) = self.resole_gaps(ctx);
        let (sizes, layout) = self.wrapped_layout(ctx, gap_x, gap_y);
        self.cache.set_computed(constraint, scale_bits, layout.size);
        let layout = Rc::new((sizes, layout));
        self.cache.with_extra(|slot: &mut Option<CachedWrap>| {
            *slot = Some(CachedWrap {
                constraint,
                scale_bits,
                layout: layout.clone(),
            });
        });
        layout
    }

    /// Resolves one wrapped child's offset and context for the retained tree,
    /// matching what [`Self::draw_wrapped`] hands the child. A wrapped flex is
    /// eager, so the child's ordinal is its index.
    pub(crate) fn retained_wrapped_child_layout<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
        index: usize,
    ) -> Option<(Vec2d, BuildContext<'a>)> {
        if self.children.get(index)?.id() != child.id() {
            return None;
        }
        let wrapped = self.wrapped_layout_cached(ctx);
        let (sizes, layout) = (&wrapped.0, &wrapped.1);
        let child_size = *sizes.get(index)?;
        let (base_x, base_y) = self.wrapped_base_offset(ctx, layout);
        let offset_x = layout.offset(index).0 + base_x;
        let offset_y = layout.offset(index).1 + base_y;
        let scale = ctx.scale.max(1.0);
        let offset = Vec2d {
            x: (offset_x * scale).round() / scale,
            y: (offset_y * scale).round() / scale,
        };
        let mut child_ctx = ctx.clone();
        child_ctx.parent_size = child_size;
        child_ctx.box_constraint = BoxConstraint {
            min_width: 0.0,
            min_height: 0.0,
            max_width: child_size.width,
            max_height: child_size.height,
        };
        child_ctx.visible_rect = ctx
            .visible_rect
            .map(|(x, y, width, height)| (x - offset_x, y - offset_y, width, height));
        Some((offset, child_ctx))
    }

    /// Cross-axis alignment shift applied to every line of a wrapped layout.
    fn wrapped_base_offset(&self, ctx: &BuildContext, layout: &WrapLayout) -> (f32, f32) {
        let extra_width = (ctx.box_constraint.max_width - layout.size.width).max(0.0);
        let extra_height = (ctx.box_constraint.max_height - layout.size.height).max(0.0);
        if matches!(self.direction, FlexDirection::Column) {
            (align_offset(self.horizontal_alignment, extra_width), 0.0)
        } else {
            (0.0, align_offset(self.vertical_alignment, extra_height))
        }
    }

    pub(crate) fn draw_wrapped(&self, ctx: &BuildContext, gap_x: f32, gap_y: f32) {
        let (sizes, layout) = self.wrapped_layout(ctx, gap_x, gap_y);
        let extra_width = (ctx.box_constraint.max_width - layout.size.width).max(0.0);
        let extra_height = (ctx.box_constraint.max_height - layout.size.height).max(0.0);
        let base_x = if matches!(self.direction, FlexDirection::Column) {
            align_offset(self.horizontal_alignment, extra_width)
        } else {
            0.0
        };
        let base_y = if matches!(self.direction, FlexDirection::Column) {
            0.0
        } else {
            align_offset(self.vertical_alignment, extra_height)
        };
        let mut draw_commands = Vec::with_capacity(self.children.len());

        for (index, child_size) in sizes.iter().copied().enumerate() {
            let Some(child) = self.children.get(index) else {
                continue;
            };
            let offset_x = layout.offsets[index].0 + base_x;
            let offset_y = layout.offsets[index].1 + base_y;
            if !ctx.is_rect_visible(offset_x, offset_y, child_size.width, child_size.height) {
                continue;
            }

            let mut child_ctx = ctx.clone();
            child_ctx.parent_size = child_size;
            child_ctx.box_constraint = BoxConstraint {
                min_width: 0.0,
                min_height: 0.0,
                max_width: child_size.width,
                max_height: child_size.height,
            };
            child_ctx.visible_rect = ctx
                .visible_rect
                .map(|(x, y, width, height)| (x - offset_x, y - offset_y, width, height));
            draw_commands.push((child.layer(), offset_x, offset_y, child_ctx, child));
        }

        draw_commands.sort_by_key(|command| command.0);
        let scale = ctx.scale.max(1.0);
        for (_, offset_x, offset_y, child_ctx, child) in draw_commands {
            let x = (offset_x * scale).round() / scale;
            let y = (offset_y * scale).round() / scale;
            ctx.canvas.save();
            ctx.canvas.translate(Vec2d { x, y });
            Self::render_child(child, &child_ctx);
            ctx.canvas.restore();
        }
    }
}

#[inline]
fn align_offset(alignment: BoxAlignment, extra: f32) -> f32 {
    match alignment {
        BoxAlignment::Start => 0.0,
        BoxAlignment::Center => extra / 2.0,
        BoxAlignment::End => extra,
    }
}

#[cfg(test)]
mod wrap_tests {
    use std::hint::black_box;
    use aimer_time::Instant;

    use aimer_attribute::size::ResolvedSize;

    use super::{FlexDirection, compute_wrap_layout};
    use crate::flex::JustifyContent;

    #[test]
    fn row_wraps_children_onto_additional_lines() {
        let children = vec![
            ResolvedSize {
                width: 60.0,
                height: 10.0,
            },
            ResolvedSize {
                width: 50.0,
                height: 20.0,
            },
            ResolvedSize {
                width: 40.0,
                height: 15.0,
            },
        ];

        let layout = compute_wrap_layout(
            &children,
            FlexDirection::Row,
            100.0,
            100.0,
            5.0,
            3.0,
            JustifyContent::Start,
        );

        assert_eq!(
            layout.size,
            ResolvedSize {
                width: 95.0,
                height: 33.0
            }
        );
        assert_eq!(layout.offsets, vec![(0.0, 0.0), (0.0, 13.0), (55.0, 13.0)]);
    }

    #[test]
    fn space_between_is_applied_to_each_wrapped_line() {
        let children = vec![
            ResolvedSize {
                width: 20.0,
                height: 10.0,
            },
            ResolvedSize {
                width: 20.0,
                height: 10.0,
            },
            ResolvedSize {
                width: 20.0,
                height: 10.0,
            },
        ];

        let layout = compute_wrap_layout(
            &children,
            FlexDirection::Row,
            50.0,
            100.0,
            0.0,
            0.0,
            JustifyContent::SpaceBetween,
        );

        assert_eq!(layout.offsets, vec![(0.0, 0.0), (30.0, 0.0), (0.0, 10.0)]);
    }

    #[test]
    fn row_wrap_keeps_gaps_after_zero_sized_children() {
        let children = vec![
            ResolvedSize {
                width: 0.0,
                height: 10.0,
            },
            ResolvedSize {
                width: 20.0,
                height: 10.0,
            },
        ];

        let layout = compute_wrap_layout(
            &children,
            FlexDirection::Row,
            100.0,
            100.0,
            5.0,
            0.0,
            JustifyContent::Start,
        );

        assert_eq!(layout.size.width, 25.0);
        assert_eq!(layout.offsets, vec![(0.0, 0.0), (5.0, 0.0)]);
    }

    #[test]
    fn column_wraps_children_onto_additional_columns() {
        let children = vec![
            ResolvedSize {
                width: 10.0,
                height: 60.0,
            },
            ResolvedSize {
                width: 20.0,
                height: 50.0,
            },
            ResolvedSize {
                width: 15.0,
                height: 40.0,
            },
        ];

        let layout = compute_wrap_layout(
            &children,
            FlexDirection::Column,
            100.0,
            100.0,
            3.0,
            5.0,
            JustifyContent::Start,
        );

        assert_eq!(
            layout.size,
            ResolvedSize {
                width: 33.0,
                height: 95.0
            }
        );
        assert_eq!(layout.offsets, vec![(0.0, 0.0), (13.0, 0.0), (13.0, 55.0)]);
    }

    #[test]
    #[ignore = "manual numeric-kernel profile"]
    fn profile_wrap_layout() {
        const MEASURED: usize = 128;
        const WARMUP: usize = 32;
        const ROUNDS: usize = 7;

        let cases = [
            (
                "row-32",
                (0..32)
                    .map(|index| ResolvedSize {
                        width: 48.0 + (index % 5) as f32 * 4.0,
                        height: 20.0 + (index % 3) as f32,
                    })
                    .collect::<Vec<_>>(),
                FlexDirection::Row,
                320.0,
                800.0,
                4.0,
                6.0,
                JustifyContent::Start,
            ),
            (
                "row-256",
                (0..256)
                    .map(|index| ResolvedSize {
                        width: 48.0 + (index % 5) as f32 * 4.0,
                        height: 20.0 + (index % 3) as f32,
                    })
                    .collect::<Vec<_>>(),
                FlexDirection::Row,
                640.0,
                800.0,
                4.0,
                6.0,
                JustifyContent::SpaceBetween,
            ),
            (
                "row-2048",
                (0..2_048)
                    .map(|index| ResolvedSize {
                        width: 48.0 + (index % 5) as f32 * 4.0,
                        height: 20.0 + (index % 3) as f32,
                    })
                    .collect::<Vec<_>>(),
                FlexDirection::Row,
                640.0,
                800.0,
                4.0,
                6.0,
                JustifyContent::SpaceBetween,
            ),
            (
                "column-32",
                (0..32)
                    .map(|index| ResolvedSize {
                        width: 48.0 + (index % 5) as f32 * 4.0,
                        height: 20.0 + (index % 3) as f32,
                    })
                    .collect::<Vec<_>>(),
                FlexDirection::Column,
                800.0,
                320.0,
                4.0,
                6.0,
                JustifyContent::Start,
            ),
            (
                "column-256",
                (0..256)
                    .map(|index| ResolvedSize {
                        width: 48.0 + (index % 5) as f32 * 4.0,
                        height: 20.0 + (index % 3) as f32,
                    })
                    .collect::<Vec<_>>(),
                FlexDirection::Column,
                800.0,
                640.0,
                4.0,
                6.0,
                JustifyContent::SpaceAround,
            ),
            (
                "column-2048",
                (0..2_048)
                    .map(|index| ResolvedSize {
                        width: 48.0 + (index % 5) as f32 * 4.0,
                        height: 20.0 + (index % 3) as f32,
                    })
                    .collect::<Vec<_>>(),
                FlexDirection::Column,
                800.0,
                640.0,
                4.0,
                6.0,
                JustifyContent::SpaceAround,
            ),
        ];

        for (name, children, direction, max_width, max_height, gap_x, gap_y, justify) in cases {
            let mut samples = Vec::with_capacity(ROUNDS);
            let mut checksum = 0.0;
            for _ in 0..ROUNDS {
                for _ in 0..WARMUP {
                    let layout = black_box(compute_wrap_layout(
                        black_box(children.as_slice()),
                        direction,
                        max_width,
                        max_height,
                        gap_x,
                        gap_y,
                        justify,
                    ));
                    checksum = black_box(
                        checksum
                            + layout.size.width
                            + layout.size.height
                            + layout.offsets.len() as f32,
                    );
                }

                let start = Instant::now();
                for _ in 0..MEASURED {
                    let layout = black_box(compute_wrap_layout(
                        black_box(children.as_slice()),
                        direction,
                        max_width,
                        max_height,
                        gap_x,
                        gap_y,
                        justify,
                    ));
                    checksum = black_box(
                        checksum
                            + layout.size.width
                            + layout.size.height
                            + layout.offsets.len() as f32,
                    );
                }
                samples.push(start.elapsed().as_secs_f64() * 1e6 / MEASURED as f64);
            }

            samples.sort_by(f64::total_cmp);
            let p50 = samples[ROUNDS / 2];
            let p95 = samples[(ROUNDS * 95).div_ceil(100) - 1];
            println!("{name}: p50 {p50:.3} us, p95 {p95:.3} us");
            assert!(checksum.is_finite());
        }
    }
}
