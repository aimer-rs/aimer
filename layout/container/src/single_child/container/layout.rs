use super::*;

impl<T: Element, D: Decoration> LayoutElement for RawContainer<T, D> {
    fn event_tree_bounds(&self) -> Option<(Vec2d, Vec2d)> {
        // These bounds follow paint transforms such as scroll offsets, which
        // can change without a layout-generation update. Keep this target
        // unbounded in the cached event index; dispatch checks the live bounds.
        None
    }

    #[inline]
    fn is_layout_stable(&self) -> bool {
        self.can_delegate_paint_islands() && self.child.is_layout_stable()
    }

    fn size(&self) -> Option<Size> {
        Some(Size {
            width: self.width,
            height: self.height,
        })
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.refresh_decoration_layout();
        let scale_bits = ctx.scale.to_bits();
        if let Some(cached) = self.cache.get_computed(ctx.box_constraint, scale_bits) {
            return cached;
        }

        let scale = ctx.scale;
        let p_w = ctx.box_constraint.max_width;
        let p_h = ctx.box_constraint.max_height;
        let threshold = 1_000_000.0f32;

        let m_left = self.margin.left.value(p_w, scale);
        let m_right = self.margin.right.value(p_w, scale);
        let m_top = self.margin.top.value(p_h, scale);
        let m_bottom = self.margin.bottom.value(p_h, scale);

        let box_width = match self.width {
            Dimension::Px(w) => w * scale,
            Dimension::Percent(p) => p_w * (p / 100.0) - (m_left + m_right),
            Dimension::Auto => p_w - (m_left + m_right),
        };

        let box_height = match self.height {
            Dimension::Px(h) => h * scale,
            Dimension::Percent(p) => p_h * (p / 100.0) - (m_top + m_bottom),
            Dimension::Auto => p_h - (m_top + m_bottom),
        };

        // When Auto dimension is unbounded (e.g. inside scrollable), derive size from
        // child
        let width_unbounded = matches!(self.width, Dimension::Auto) && box_width > threshold;
        let height_unbounded = matches!(self.height, Dimension::Auto) && box_height > threshold;

        let result = if width_unbounded || height_unbounded {
            let capped_w = box_width.min(threshold);
            let capped_h = box_height.min(threshold);

            let p_left = self.padding.left.value(capped_w, scale);
            let p_right = self.padding.right.value(capped_w, scale);
            let p_top = self.padding.top.value(capped_h, scale);
            let p_bottom = self.padding.bottom.value(capped_h, scale);

            let [bl, bt, br, bb] = self.decoration_insets(capped_w, capped_h, scale);

            let mut child_ctx = ctx.clone();
            child_ctx.box_constraint.max_width = if width_unbounded {
                f32::MAX
            } else {
                (box_width - p_left - bl - p_right - br).max(0.0)
            };
            child_ctx.box_constraint.max_height = if height_unbounded {
                f32::MAX
            } else {
                (box_height - p_top - bt - p_bottom - bb).max(0.0)
            };
            // The child is laid out in the space inside the padding, so a bounded
            // axis is measured against that parent, as `local_v2_child_context`
            // hands it down. A parent size left at the container's own would be
            // cached by the layouts below, which are keyed by the constraint
            // alone. An unbounded axis keeps the parent it was offered: a scroll
            // view leaves the viewport there for children that fill their parent.
            child_ctx.parent_size = ResolvedSize {
                width: if width_unbounded {
                    ctx.parent_size.width
                } else {
                    child_ctx.box_constraint.max_width
                },
                height: if height_unbounded {
                    ctx.parent_size.height
                } else {
                    child_ctx.box_constraint.max_height
                },
            };
            let child_size = self.child.computed_size(&child_ctx);

            let final_w = if width_unbounded {
                child_size.width + p_left + p_right + bl + br + m_left + m_right
            } else {
                box_width + m_left + m_right
            };
            let final_h = if height_unbounded {
                child_size.height + p_top + p_bottom + bt + bb + m_top + m_bottom
            } else {
                box_height + m_top + m_bottom
            };

            ResolvedSize {
                width: final_w.max(0.0),
                height: final_h.max(0.0),
            }
        } else {
            ResolvedSize {
                width: (box_width + m_left + m_right).max(0.0),
                height: (box_height + m_top + m_bottom).max(0.0),
            }
        };
        self.cache
            .set_computed(ctx.box_constraint, scale_bits, result);
        result
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.refresh_decoration_layout();
        let scale_bits = ctx.scale.to_bits();
        if let Some(cached) = self.cache.get_content(ctx.box_constraint, scale_bits) {
            return cached;
        }

        let scale = ctx.scale;
        let p_w = ctx.box_constraint.max_width;
        let p_h = ctx.box_constraint.max_height;
        let threshold = 1_000_000.0f32;

        let m_left = self.margin.left.value(p_w, scale);
        let m_right = self.margin.right.value(p_w, scale);
        let m_top = self.margin.top.value(p_h, scale);
        let m_bottom = self.margin.bottom.value(p_h, scale);

        let box_width = match self.width {
            Dimension::Px(w) => w * scale,
            Dimension::Percent(p) => p_w * (p / 100.0) - (m_left + m_right),
            Dimension::Auto => p_w - (m_left + m_right),
        };
        let box_height = match self.height {
            Dimension::Px(h) => h * scale,
            Dimension::Percent(p) => p_h * (p / 100.0) - (m_top + m_bottom),
            Dimension::Auto => p_h - (m_top + m_bottom),
        };

        let width_unbounded = matches!(self.width, Dimension::Auto) && box_width > threshold;
        let height_unbounded = matches!(self.height, Dimension::Auto) && box_height > threshold;

        let b_w = box_width.max(0.0);
        let b_h = box_height.max(0.0);
        let capped_w = b_w.min(threshold);
        let capped_h = b_h.min(threshold);

        let p_left = self.padding.left.value(capped_w, scale);
        let p_right = self.padding.right.value(capped_w, scale);
        let p_top = self.padding.top.value(capped_h, scale);
        let p_bottom = self.padding.bottom.value(capped_h, scale);

        let [b_left, b_top, b_right, b_bottom] = self.decoration_insets(capped_w, capped_h, scale);

        let result = if width_unbounded || height_unbounded {
            let mut child_ctx = ctx.clone();
            child_ctx.box_constraint.max_width = if width_unbounded {
                f32::MAX
            } else {
                (b_w - p_left - b_left - p_right - b_right).max(0.0)
            };
            child_ctx.box_constraint.max_height = if height_unbounded {
                f32::MAX
            } else {
                (b_h - p_top - b_top - p_bottom - b_bottom).max(0.0)
            };
            // See `computed_size`: a bounded axis is measured against the parent
            // the child is laid out in, an unbounded one keeps the viewport.
            child_ctx.parent_size = ResolvedSize {
                width: if width_unbounded {
                    ctx.parent_size.width
                } else {
                    child_ctx.box_constraint.max_width
                },
                height: if height_unbounded {
                    ctx.parent_size.height
                } else {
                    child_ctx.box_constraint.max_height
                },
            };
            let child_size = self.child.computed_size(&child_ctx);

            ResolvedSize {
                width: if width_unbounded {
                    child_size.width
                } else {
                    (b_w - p_left - p_right - b_left - b_right).max(0.0)
                },
                height: if height_unbounded {
                    child_size.height
                } else {
                    (b_h - p_top - p_bottom - b_top - b_bottom).max(0.0)
                },
            }
        } else {
            ResolvedSize {
                width: (b_w - p_left - p_right - b_left - b_right).max(0.0),
                height: (b_h - p_top - p_bottom - b_top - b_bottom).max(0.0),
            }
        };
        self.cache
            .set_content(ctx.box_constraint, scale_bits, result);
        result
    }

    fn get_size_from_child(&self) -> Option<Size> {
        let mut size = self.child.get_size_from_child().unwrap_or_default();

        let mut m_w: f32 = 0.0;
        let mut m_h: f32 = 0.0;
        let mut p_w: f32 = 0.0;
        let mut p_h: f32 = 0.0;
        let mut b_w: f32 = 0.0;
        let mut b_h: f32 = 0.0;

        // Note: For get_size_from_child, we don't have a parent size to resolve
        // percentages, so we can only accurately add Px values. Percentages
        // will be ignored or should be handled by the layout system during
        // actual resolution.

        // `computed_size` adds the margin to the box, so a size read from the
        // content has to include it or a parent that hugs it clips the child.
        if let Spacing::Px(v) = self.margin.left {
            m_w += v as f32;
        }
        if let Spacing::Px(v) = self.margin.right {
            m_w += v as f32;
        }
        if let Spacing::Px(v) = self.margin.top {
            m_h += v as f32;
        }
        if let Spacing::Px(v) = self.margin.bottom {
            m_h += v as f32;
        }

        if let Spacing::Px(v) = self.padding.left {
            p_w += v as f32;
        }
        if let Spacing::Px(v) = self.padding.right {
            p_w += v as f32;
        }
        if let Spacing::Px(v) = self.padding.top {
            p_h += v as f32;
        }
        if let Spacing::Px(v) = self.padding.bottom {
            p_h += v as f32;
        }

        let [left, top, right, bottom] = self.decoration_insets(0.0, 0.0, 1.0);
        b_w += left + right;
        b_h += top + bottom;

        if let Dimension::Px(w) = self.width {
            size.width = Dimension::Px(w + m_w);
        } else {
            size.width = match size.width {
                Dimension::Px(v) => Dimension::Px(v + m_w + p_w + b_w),
                other => other,
            };
        }

        if let Dimension::Px(h) = self.height {
            size.height = Dimension::Px(h + m_h);
        } else {
            size.height = match size.height {
                Dimension::Px(v) => Dimension::Px(v + m_h + p_h + b_h),
                other => other,
            };
        }

        Some(size)
    }

    fn invalidate_layout(&self) {
        self.cache.invalidate();
        self.child.invalidate_layout();
    }

    /// The box the container answers pointer events in: its own, and any further
    /// reach its child reports.
    ///
    /// The dispatcher offers a pointer event only to elements whose bounds
    /// contain it, so a container that reported its box alone made everything
    /// its child reaches outside that box unreachable: the outer half of a
    /// resize handle at the edge of a panel, for instance, which the child
    /// extends past the border on purpose. A child inside the box adds nothing.
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        let (start, end) = self.bounds.pos_start_end()?;
        let Some((child_start, child_end)) = self.child.pos_start_end() else {
            return Some((start, end));
        };
        Some((
            Vec2d {
                x: start.x.min(child_start.x),
                y: start.y.min(child_start.y),
            },
            Vec2d {
                x: end.x.max(child_end.x),
                y: end.y.max(child_end.y),
            },
        ))
    }
}
