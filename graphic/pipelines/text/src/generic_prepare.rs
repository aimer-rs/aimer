use super::*;
use crate::backend::GpuBackend;

impl<B: GpuBackend> TextPipelineV2<B> {
    /// Prepares glyphs and uploads atlases and instances through `backend`.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare(
        &mut self,
        backend: &B,
        width: u32,
        height: u32,
        is_srgb: bool,
        requests: &[TextDrawRequest],
        decorations: &[TextDecorationDraw],
    ) {
        self.prepare_inner_generic(backend, width, height, is_srgb, requests, decorations, None);
    }

    /// Runs [`prepare`](Self::prepare) and returns a CPU-stage timing snapshot.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_profiled(
        &mut self,
        backend: &B,
        width: u32,
        height: u32,
        is_srgb: bool,
        requests: &[TextDrawRequest],
        decorations: &[TextDecorationDraw],
    ) -> TextPreparationProfile {
        let mut profile = TextPreparationProfile::default();
        self.prepare_inner_generic(
            backend,
            width,
            height,
            is_srgb,
            requests,
            decorations,
            Some(&mut profile),
        );
        profile
    }

    #[inline]
    fn request_has_layout_miss_generic(&self, req: &TextDrawRequest) -> bool {
        let synthesized: [RichTextSpan; 1];
        let spans: &[RichTextSpan] = if req.spans.is_empty() {
            synthesized = [RichTextSpan::new(req.text.clone())];
            &synthesized
        } else {
            &req.spans
        };

        spans.iter().any(|span| {
            let font_size = span.font_size.unwrap_or(req.font_size);
            let font_weight = span
                .font_weight
                .or(req.font_weight)
                .unwrap_or(FontWeight::Normal.numeric());
            let layout_width = if req.writing_mode.is_vertical() {
                // Vertical-rl uses the width to anchor the rightmost column
                // even when overflow is clip-only and no wrapping is done.
                req.bounds_width
            } else {
                match req.overflow {
                    TextOverflowMode::Wrap | TextOverflowMode::Ellipsis => req.bounds_width,
                    TextOverflowMode::Clip => 0.0,
                }
            };
            let layout_height = if req.writing_mode.is_vertical() {
                match req.overflow {
                    TextOverflowMode::Wrap | TextOverflowMode::Ellipsis => req.bounds_height,
                    TextOverflowMode::Clip => 0.0,
                }
            } else {
                0.0
            };
            let keys = span_layout_keys_with_writing_mode(
                &self.shaping_cache,
                &span.text,
                font_size,
                req.font_family,
                req.font_style,
                font_weight,
                req.language,
                layout_width,
                layout_height,
                req.writing_mode,
            );
            self.layout_cache
                .peek_with_fallback(&keys.primary, keys.fallback.as_ref())
                .is_none()
        })
    }

    fn prepare_content_generic(
        &mut self,
        requests: &[&TextDrawRequest],
        mut profile: Option<&mut TextPreparationProfile>,
    ) -> bool {
        let clear_shaping_cache = self.shaping_cache.len() > Self::SHAPING_CACHE_CAPACITY;

        let mut shaping_batch = PreparationBatch::new();
        let mut layout_batch = PreparationBatch::new();
        let key_started = profile.is_some().then(Instant::now);
        for req in requests {
            let synthesized: [RichTextSpan; 1];
            let spans: &[RichTextSpan] = if req.spans.is_empty() {
                synthesized = [RichTextSpan::new(req.text.clone())];
                &synthesized
            } else {
                &req.spans
            };

            for span in spans {
                let font_size = span.font_size.unwrap_or(req.font_size);
                let font_weight = span
                    .font_weight
                    .or(req.font_weight)
                    .unwrap_or(FontWeight::Normal.numeric());
                let layout_width = if req.writing_mode.is_vertical() {
                    req.bounds_width
                } else {
                    match req.overflow {
                        TextOverflowMode::Wrap | TextOverflowMode::Ellipsis => req.bounds_width,
                        TextOverflowMode::Clip => 0.0,
                    }
                };
                let layout_height = if req.writing_mode.is_vertical() {
                    match req.overflow {
                        TextOverflowMode::Wrap | TextOverflowMode::Ellipsis => req.bounds_height,
                        TextOverflowMode::Clip => 0.0,
                    }
                } else {
                    0.0
                };
                let keys = span_layout_keys_with_writing_mode(
                    &self.shaping_cache,
                    &span.text,
                    font_size,
                    req.font_family,
                    req.font_style,
                    font_weight,
                    req.language,
                    layout_width,
                    layout_height,
                    req.writing_mode,
                );
                if self
                    .layout_cache
                    .get_with_fallback(&keys.primary, keys.fallback.as_ref())
                    .is_some()
                {
                    continue;
                }

                if clear_shaping_cache || !self.shaping_cache.contains_key(&keys.shaping_key) {
                    shaping_batch.push(
                        keys.shaping_key.clone(),
                        ShapingInput {
                            text: span.text.clone(),
                            font_size,
                            font_family: req.font_family,
                            font_style: req.font_style,
                            font_weight,
                            language: req.language,
                            writing_mode: req.writing_mode,
                        },
                    );
                }
                layout_batch.push(
                    keys.primary,
                    LayoutInput {
                        shaping_key: keys.shaping_key,
                        layout_width: keys.layout_width,
                        layout_height: keys.layout_height,
                    },
                );
            }
        }

        if let (Some(profile), Some(started)) = (profile.as_deref_mut(), key_started) {
            profile.key_construction += started.elapsed();
        }

        let fallback_started = profile.is_some().then(Instant::now);
        if !shaping_batch.jobs().is_empty() {
            for req in requests {
                let synthesized: [RichTextSpan; 1];
                let spans: &[RichTextSpan] = if req.spans.is_empty() {
                    synthesized = [RichTextSpan::new(req.text.clone())];
                    &synthesized
                } else {
                    &req.spans
                };
                for span in spans {
                    let font_size = span.font_size.unwrap_or(req.font_size);
                    let font_weight = span
                        .font_weight
                        .or(req.font_weight)
                        .unwrap_or(FontWeight::Normal.numeric());
                    self.rasterizer.warm_fallbacks_for_text(
                        &span.text,
                        font_size,
                        req.font_family,
                        FontWeight::Value(u32::from(font_weight)),
                        req.font_style,
                        req.language,
                    );
                }
            }
        }
        if let (Some(profile), Some(started)) = (profile.as_deref_mut(), fallback_started) {
            let fallback_elapsed = started.elapsed();
            profile.fallback_resolution += fallback_elapsed;
            profile.fallback_and_shaping += fallback_elapsed;
        }

        let snapshot_started = profile.is_some().then(Instant::now);
        let font_snapshot = self.rasterizer.font_snapshot();
        if let (Some(profile), Some(started)) = (profile.as_deref_mut(), snapshot_started) {
            let snapshot_elapsed = started.elapsed();
            profile.font_snapshot += snapshot_elapsed;
            profile.fallback_and_shaping += snapshot_elapsed;
        }

        let shaping_started = profile.is_some().then(Instant::now);
        let shaping_jobs = shaping_batch.into_shared_jobs();
        let shaping_snapshot = font_snapshot.clone();
        let shaping_results = self.executor.execute_persistent_with_context(
            shaping_jobs.clone(),
            move || GlyphPreparationContext::new(shaping_snapshot.clone()),
            move |context, job| {
                let input = &job.input;
                Some(prepare_shaped_text_with_writing_mode(
                    context,
                    &input.text,
                    input.font_size,
                    input.font_family,
                    FontWeight::Value(u32::from(input.font_weight)),
                    input.font_style,
                    input.language,
                    input.writing_mode,
                ))
            },
        );
        let Ok(shaping_results) = shaping_results else {
            return false;
        };
        let Ok(prepared_shaping) = PreparationBatch::<ShapingCacheKey, ShapingInput>::merge_jobs(
            &shaping_jobs,
            shaping_results,
        ) else {
            return false;
        };
        let prepared_shaping = prepared_shaping
            .into_iter()
            .map(|(key, shaped)| (key, Arc::new(shaped)))
            .collect::<HashMap<_, _>>();

        if let (Some(profile), Some(started)) = (profile.as_deref_mut(), shaping_started) {
            let shaping_elapsed = started.elapsed();
            profile.shaping += shaping_elapsed;
            profile.fallback_and_shaping += shaping_elapsed;
            profile.shaping_jobs += shaping_jobs.len();
        }

        // Positioning is pure arithmetic over the shaped clusters — the pixel
        // box of every glyph was baked in at shaping time — so layout jobs need
        // no rasterizer or font context. Transfer an owned, Arc-backed shaped
        // result to the persistent workers so the callback does not borrow the
        // pipeline and cached shaping results are never copied.
        let layout_started = profile.is_some().then(Instant::now);
        let mut owned_layout_jobs = Vec::with_capacity(layout_batch.jobs().len());
        for job in layout_batch.jobs() {
            let shaped = prepared_shaping
                .get(&job.input.shaping_key)
                .or_else(|| {
                    (!clear_shaping_cache)
                        .then(|| self.shaping_cache.get(&job.input.shaping_key))
                        .flatten()
                })
                .cloned();
            let Some(shaped) = shaped else {
                return false;
            };
            owned_layout_jobs.push(IndexedJob::new(
                job.order,
                job.key.clone(),
                OwnedLayoutInput {
                    shaped,
                    layout_width: job.input.layout_width,
                    layout_height: job.input.layout_height,
                },
            ));
        }
        let owned_layout_jobs = Arc::<[_]>::from(owned_layout_jobs);
        let layout_results = self.executor.execute_persistent_with_context(
            owned_layout_jobs.clone(),
            || (),
            move |(), job| {
                Some(layout_shaped_text_result_with_bounds(
                    &job.input.shaped,
                    0.0,
                    0.0,
                    job.input.layout_width,
                    job.input.layout_height,
                ))
            },
        );
        let Ok(layout_results) = layout_results else {
            return false;
        };
        let Ok(prepared_layouts) = PreparationBatch::<LayoutCacheKey, OwnedLayoutInput>::merge_jobs(
            &owned_layout_jobs,
            layout_results,
        ) else {
            return false;
        };
        let prepared_layouts = prepared_layouts.into_iter().collect::<HashMap<_, _>>();

        if let (Some(profile), Some(started)) = (profile.as_deref_mut(), layout_started) {
            profile.layout += started.elapsed();
            profile.layout_jobs += owned_layout_jobs.len();
        }

        let mut fresh_glyphs = Vec::new();
        let mut queued_glyphs = HashSet::new();
        let glyph_key_started = profile.is_some().then(Instant::now);
        for req in requests {
            let synthesized: [RichTextSpan; 1];
            let spans: &[RichTextSpan] = if req.spans.is_empty() {
                synthesized = [RichTextSpan::new(req.text.clone())];
                &synthesized
            } else {
                &req.spans
            };

            for span in spans {
                let font_size = span.font_size.unwrap_or(req.font_size);
                let font_weight = span
                    .font_weight
                    .or(req.font_weight)
                    .unwrap_or(FontWeight::Normal.numeric());
                let layout_width = if req.writing_mode.is_vertical() {
                    req.bounds_width
                } else {
                    match req.overflow {
                        TextOverflowMode::Wrap | TextOverflowMode::Ellipsis => req.bounds_width,
                        TextOverflowMode::Clip => 0.0,
                    }
                };
                let layout_height = if req.writing_mode.is_vertical() {
                    match req.overflow {
                        TextOverflowMode::Wrap | TextOverflowMode::Ellipsis => req.bounds_height,
                        TextOverflowMode::Clip => 0.0,
                    }
                } else {
                    0.0
                };
                let keys = span_layout_keys_with_writing_mode(
                    &self.shaping_cache,
                    &span.text,
                    font_size,
                    req.font_family,
                    req.font_style,
                    font_weight,
                    req.language,
                    layout_width,
                    layout_height,
                    req.writing_mode,
                );
                let Some(positioned) = prepared_layouts
                    .get(&keys.primary)
                    .map(|layout| layout.glyphs.as_slice())
                    .or_else(|| {
                        self.layout_cache
                            .peek_with_fallback(&keys.primary, keys.fallback.as_ref())
                    })
                else {
                    return false;
                };

                for glyph in positioned {
                    let key = glyph.glyph_key;
                    let needs_bitmap =
                        self.atlas.get(&key).is_none() && self.color_atlas.get(&key).is_none();
                    if self.rasterizer.needs_prepared_glyph(key, needs_bitmap)
                        && queued_glyphs.insert(key)
                    {
                        fresh_glyphs.push((key, glyph.font_size));
                    }
                }
            }
        }

        if let (Some(profile), Some(started)) = (profile.as_deref_mut(), glyph_key_started) {
            profile.key_construction += started.elapsed();
        }

        // Glyphs are prepared in runs sharing a face and a size, so the work
        // that belongs to the face — mapping the file, parsing its tables,
        // building the scaler and reading the metrics — is done once per run
        // instead of once per glyph. The runs are bounded, so a page of one
        // face at one size still spreads across the workers.
        let glyph_started = profile.is_some().then(Instant::now);
        let mut glyph_batch = PreparationBatch::new();
        for (order, run) in glyph_runs(fresh_glyphs).into_iter().enumerate() {
            glyph_batch.push(order, run);
        }

        let glyph_jobs = glyph_batch.into_shared_jobs();
        let glyph_results = self.executor.execute_persistent_with_context(
            glyph_jobs.clone(),
            move || GlyphPreparationContext::new(font_snapshot.clone()),
            move |context, job| Some(context.prepare_glyph_run(&job.input)),
        );
        let Ok(glyph_results) = glyph_results else {
            return false;
        };
        let Ok(prepared_glyphs) = PreparationBatch::merge_jobs(&glyph_jobs, glyph_results) else {
            return false;
        };

        if clear_shaping_cache {
            self.shaping_cache.clear();
        }
        self.shaping_cache.extend(prepared_shaping);
        self.layout_cache.extend(prepared_layouts);
        for (_, glyphs) in prepared_glyphs {
            for (key, glyph) in glyphs {
                self.rasterizer.commit_prepared_glyph(key, glyph);
            }
        }

        if let (Some(profile), Some(started)) = (profile.as_deref_mut(), glyph_started) {
            profile.glyph_preparation += started.elapsed();
            profile.glyph_jobs += glyph_jobs.len();
        }

        true
    }

    #[allow(clippy::too_many_arguments)]
    fn build_text_paint_template_generic(
        &mut self,
        backend: &B,
        request: &TextDrawRequest,
        keys: &SpanLayoutKeys,
        font_weight: u16,
        italic: bool,
        mut profile: Option<&mut TextPreparationProfile>,
    ) -> Option<Arc<CachedTextPaint>> {
        let positioned = self
            .layout_cache
            .peek_with_fallback(&keys.primary, keys.fallback.as_ref())?;
        let line_offsets = if request.writing_mode.is_vertical() {
            None
        } else {
            match request.horizontal_align {
                TextHorizontalAlign::Left => None,
                _ => Some(line_alignment_offsets(
                    &positioned_line_widths(positioned),
                    request.bounds_width,
                    request.horizontal_align,
                )),
            }
        };
        let shadow = sanitize_shadow(request.shadow);
        let sample_count: usize = shadow.map_or(0, |shadow| if shadow.blur == 0.0 { 1 } else { 8 });
        let shadow_coverage_exponent = shadow
            .map(|shadow| coverage_exponent(shadow.color.to_unorm_array()))
            .unwrap_or(1.0);
        let mut glyphs = Vec::with_capacity(
            positioned
                .len()
                .saturating_mul(sample_count.saturating_add(1)),
        );

        for pg in positioned {
            let key = pg.glyph_key;
            let (region, target_color_list) = if let Some(region) = self.atlas.get(&key) {
                (region, false)
            } else if let Some(region) = self.color_atlas.get(&key) {
                (region, true)
            } else {
                let atlas_population_started = profile.is_some().then(Instant::now);
                let rg = self.rasterizer.rasterize_bitmap_key(key, pg.font_size);
                let (is_color, glyph_width, glyph_height) = (rg.is_color, rg.width, rg.height);
                let region = if is_color {
                    self.color_atlas.get_or_insert(
                        backend,
                        key,
                        glyph_width,
                        glyph_height,
                        &rg.bitmap,
                    )
                } else {
                    self.atlas
                        .get_or_insert(backend, key, glyph_width, glyph_height, &rg.bitmap)
                };
                self.rasterizer.release_bitmap(key);
                if let (Some(profile), Some(started)) =
                    (profile.as_deref_mut(), atlas_population_started)
                {
                    profile.atlas_population += started.elapsed();
                }
                (region, is_color)
            };

            let size = glyph_quad_size((region.width, region.height));
            let line_offset = line_offsets
                .as_ref()
                .map_or(0.0, |offsets| offsets[pg.line_index]);
            let local_position = [pg.x + line_offset, pg.y];
            let skew = if italic { 0.25 } else { 0.0 };

            if let Some(shadow) = shadow {
                for sample in 0..sample_count {
                    let (blur_x, blur_y) = if sample_count == 1 {
                        (0.0, 0.0)
                    } else {
                        let angle = sample as f32 * std::f32::consts::TAU / sample_count as f32;
                        (angle.cos() * shadow.blur, angle.sin() * shadow.blur)
                    };
                    glyphs.push(CachedPaintGlyph {
                        local_position,
                        offset: [shadow.offset_x + blur_x, shadow.offset_y + blur_y],
                        size,
                        region,
                        color_atlas: target_color_list,
                        skew,
                        kind: CachedPaintKind::Shadow,
                    });
                }
            }

            glyphs.push(CachedPaintGlyph {
                local_position,
                offset: [0.0, 0.0],
                size,
                region,
                color_atlas: target_color_list,
                skew,
                kind: CachedPaintKind::Foreground,
            });
            if !target_color_list {
                if let Some(plan) = self.rasterizer.synthetic_weight_plan_for_codepoint(
                    key,
                    font_weight,
                    pg.font_size,
                    pg.codepoint,
                ) {
                    for &offset in plan.extra_offsets() {
                        glyphs.push(CachedPaintGlyph {
                            local_position,
                            offset: [offset, 0.0],
                            size,
                            region,
                            color_atlas: false,
                            skew,
                            kind: CachedPaintKind::Foreground,
                        });
                    }
                }
            }
        }

        let (advance_x, advance_y) = if request.writing_mode.is_vertical() {
            (
                0.0,
                positioned
                    .iter()
                    .map(|glyph| glyph.y + glyph.height as f32)
                    .max_by(f32::total_cmp)
                    .unwrap_or(0.0)
                    .max(0.0),
            )
        } else {
            positioned
                .last()
                .map_or((0.0, 0.0), |last| (last.x + last.width as f32, last.y))
        };

        Some(Arc::new(CachedTextPaint {
            glyphs,
            advance_x,
            advance_y,
            shadow,
            shadow_coverage_exponent,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_inner_generic(
        &mut self,
        backend: &B,
        width: u32,
        height: u32,
        is_srgb: bool,
        requests: &[TextDrawRequest],
        decorations: &[TextDecorationDraw],
        mut profile: Option<&mut TextPreparationProfile>,
    ) {
        let started = AnimInstant::now();
        let profile_started = profile.is_some().then(Instant::now);

        let atlas_generation = self.atlas.generation();
        let color_atlas_generation = self.color_atlas.generation();
        self.paint_cache.ensure_generations(
            atlas_generation,
            color_atlas_generation,
            FontRegistry::revision(),
        );
        let frame_cache_hit = !self.postponed_preparation
            && self.prepared_frame_generation == self.frame_generation
            && self.prepared_frame.as_ref().is_some_and(|cached| {
                cached.matches(
                    width,
                    height,
                    is_srgb,
                    atlas_generation,
                    color_atlas_generation,
                    FontRegistry::revision(),
                    requests,
                    decorations,
                )
            });
        if frame_cache_hit {
            if let Some(profile) = profile.as_deref_mut() {
                profile.cache_hit = true;
                if let Some(started) = profile_started {
                    profile.total = started.elapsed();
                }
            }
            return;
        }
        self.frame_generation = self.frame_generation.wrapping_add(1);

        // A scroll viewport hands its child more than it can show, so that a
        // line is prepared a few frames before it scrolls in instead of on the
        // frame its edge crosses the boundary. That moves the work rather than
        // removing it: during a fling fresh content arrives every frame, and
        // preparing all of it turns one visible stall into a continuous one.
        // What the frame owes the screen is prepared whatever it costs; the
        // rest is offered the time that is left, and what does not fit waits
        // for the frame this asks for. Dropping a visible glyph shows a blank,
        // dropping one the clip discards shows nothing at all.
        //
        // The same split bounds the *drawing* below: a request that cannot
        // show a pixel builds no glyph instances and reserves no atlas room,
        // so scrolling a text-heavy document costs the frame what is on
        // screen, not what the viewport asked for ahead of itself.

        // A layout the frame reads is stamped as used; opening the frame is
        // also when a cache that outgrew its capacity sheds the entries no
        // recent frame read — a resize's flood of transient widths, a closed
        // screen's lines — without touching the current screen's layouts.
        let request_analysis_started = profile.is_some().then(Instant::now);
        self.layout_cache.begin_frame();

        // A frame whose surface size changed is one step of a live resize:
        // every wrapped layout is keyed by a width that will be different
        // again next frame.
        let resizing = self.last_prepared_surface != (width, height);
        self.last_prepared_surface = (width, height);

        let surface = (width as f32, height as f32);
        let mut on_screen = Vec::with_capacity(requests.len());
        let mut ahead_of_view = Vec::new();
        let mut visible = vec![true; requests.len()];
        for (index, req) in requests.iter().enumerate() {
            // Glyphs can render slightly outside the box the request declares:
            // an ascender or italic left-bearing reaches above/left of the
            // origin, and a tight box can clip glyphs that overflow it. The
            // coarse on-screen test below would then drop a request whose
            // glyphs have already scrolled into (or not yet out of) the clip.
            // Inflate the box by the request's largest font size so the test
            // only ever errs toward keeping text — `glyph_intersects_clip`
            // does the precise per-glyph check afterwards. Non-positive extents
            // are left alone so `known_extent` still reads them as unbounded.
            let glyph_pad = req
                .spans
                .iter()
                .map(|span| span.font_size.unwrap_or(req.font_size))
                .fold(req.font_size, f32::max);
            let shadow_pad = req.shadow.map_or(0.0, shadow_padding);
            let pad = glyph_pad + shadow_pad;
            let bounds = [
                req.x - pad,
                req.y - pad,
                if req.bounds_width > 0.0 {
                    req.bounds_width + 2.0 * pad
                } else {
                    req.bounds_width
                },
                if req.bounds_height > 0.0 {
                    req.bounds_height + 2.0 * pad
                } else {
                    req.bounds_height
                },
            ];
            if request_is_on_screen(bounds, req.clip_rect, surface) {
                on_screen.push(req);
            } else {
                visible[index] = false;
                // The scroll cache window deliberately includes nearby
                // content, but a cache hit must not start shaping/layout
                // executor work just because it is off-screen.
                if self.request_has_layout_miss_generic(req) {
                    ahead_of_view.push((index, req));
                }
            }
        }

        if let (Some(profile), Some(started)) = (profile.as_deref_mut(), request_analysis_started) {
            profile.request_analysis += started.elapsed();
        }

        if !self.prepare_content_generic(&on_screen, profile.as_deref_mut()) {
            return;
        }

        self.postponed_preparation = false;
        if !ahead_of_view.is_empty() {
            if resizing {
                // Laying the tail out now would spend the budget on layouts
                // keyed by a width the next resize frame invalidates, and
                // flood the layout cache with entries nothing will read. The
                // tail is postponed instead: a later real frame prepares it
                // at the width that will actually be drawn.
                self.postponed_preparation = true;
            } else {
                let mut chunk_requests = Vec::with_capacity(PREPARATION_CHUNK);
                let prepared = prepare_ahead_of_view(
                    &ahead_of_view,
                    PreparationBudget::starting_at(started, PREPARATION_BUDGET),
                    AnimInstant::now,
                    |chunk| {
                        chunk_requests.clear();
                        chunk_requests.extend(chunk.iter().map(|(_, req)| *req));
                        self.prepare_content_generic(&chunk_requests, profile.as_deref_mut())
                    },
                );

                self.postponed_preparation = prepared < ahead_of_view.len();
            }
        }

        let post_content_key_started = profile.is_some().then(Instant::now);
        self.visible_span_ranges.clear();
        self.visible_span_ranges.resize(requests.len(), 0..0);
        self.visible_span_keys.clear();
        for (index, req) in requests.iter().enumerate() {
            // An off-screen request contributes no pixel this frame, so it
            // reserves no atlas room either; its glyphs claim theirs on the
            // frame they scroll in, from the bitmaps preparation cached.
            if !visible[index] {
                continue;
            }

            let start = self.visible_span_keys.len();
            let synthesized: [RichTextSpan; 1];
            let spans: &[RichTextSpan] = if req.spans.is_empty() {
                synthesized = [RichTextSpan::new(req.text.clone())];
                &synthesized
            } else {
                &req.spans
            };

            for span in spans {
                let font_size = span.font_size.unwrap_or(req.font_size);
                let font_weight = span
                    .font_weight
                    .or(req.font_weight)
                    .unwrap_or(FontWeight::Normal.numeric());
                let layout_width = if req.writing_mode.is_vertical() {
                    req.bounds_width
                } else {
                    match req.overflow {
                        TextOverflowMode::Wrap | TextOverflowMode::Ellipsis => req.bounds_width,
                        TextOverflowMode::Clip => 0.0,
                    }
                };
                let layout_height = if req.writing_mode.is_vertical() {
                    match req.overflow {
                        TextOverflowMode::Wrap | TextOverflowMode::Ellipsis => req.bounds_height,
                        TextOverflowMode::Clip => 0.0,
                    }
                } else {
                    0.0
                };
                self.visible_span_keys
                    .push(span_layout_keys_with_writing_mode(
                        &self.shaping_cache,
                        &span.text,
                        font_size,
                        req.font_family,
                        req.font_style,
                        font_weight,
                        req.language,
                        layout_width,
                        layout_height,
                        req.writing_mode,
                    ));
            }
            self.visible_span_ranges[index] = start..self.visible_span_keys.len();
        }

        if let (Some(profile), Some(started)) = (profile.as_deref_mut(), post_content_key_started) {
            profile.key_construction += started.elapsed();
        }

        self.alpha_glyph_descriptors.clear();
        self.color_glyph_descriptors.clear();
        self.seen_glyphs.clear();
        for (index, range) in self.visible_span_ranges.iter().enumerate() {
            if !visible[index] {
                continue;
            }

            for keys in &self.visible_span_keys[range.clone()] {
                // The stamping read: this is the loop that walks exactly the
                // layouts the frame draws, so it is where the working set is
                // marked as alive.
                let Some(positioned) = self
                    .layout_cache
                    .get_with_fallback(&keys.primary, keys.fallback.as_ref())
                else {
                    return;
                };

                for glyph in positioned {
                    let key = glyph.glyph_key;
                    if !self.seen_glyphs.insert(key) {
                        continue;
                    }
                    let Some((is_color, glyph_width, glyph_height)) =
                        self.rasterizer.cached_glyph_descriptor(key)
                    else {
                        return;
                    };
                    let descriptor = (key, glyph_width, glyph_height);
                    if is_color {
                        self.color_glyph_descriptors.push(descriptor);
                    } else {
                        self.alpha_glyph_descriptors.push(descriptor);
                    }
                }
            }
        }

        let atlas_planning_started = profile.is_some().then(Instant::now);
        let alpha_needs_plan = self.planned_alpha_atlas_generation != self.atlas.generation()
            || self.planned_alpha_descriptors != self.alpha_glyph_descriptors;
        let color_needs_plan = self.planned_color_atlas_generation != self.color_atlas.generation()
            || self.planned_color_descriptors != self.color_glyph_descriptors;
        if alpha_needs_plan {
            let alpha_plan = self.atlas.plan_batch_generic(&self.alpha_glyph_descriptors);
            if alpha_plan == BatchCapacityPlan::Reject {
                return;
            }
            self.atlas.apply_batch_plan_generic(alpha_plan);
            self.planned_alpha_descriptors
                .clone_from(&self.alpha_glyph_descriptors);
            self.planned_alpha_atlas_generation = self.atlas.generation();
        }
        if color_needs_plan {
            let color_plan = self
                .color_atlas
                .plan_batch_generic(&self.color_glyph_descriptors);
            if color_plan == BatchCapacityPlan::Reject {
                return;
            }
            self.color_atlas.apply_batch_plan_generic(color_plan);
            self.planned_color_descriptors
                .clone_from(&self.color_glyph_descriptors);
            self.planned_color_atlas_generation = self.color_atlas.generation();
        }

        // Planning can repack a full atlas before the assembly loop starts.
        // Refresh the paint cache after that decision as well as at frame
        // entry, otherwise a reset could leave cached quads pointing at the
        // old glyph coordinates for this very frame.
        self.paint_cache.ensure_generations(
            self.atlas.generation(),
            self.color_atlas.generation(),
            FontRegistry::revision(),
        );

        if let (Some(profile), Some(started)) = (profile.as_deref_mut(), atlas_planning_started) {
            profile.atlas_planning += started.elapsed();
            profile.alpha_glyphs += self.alpha_glyph_descriptors.len();
            profile.color_glyphs += self.color_glyph_descriptors.len();
        }

        self.instances.clear();
        self.color_instances.clear();
        self.decoration_instances.clear();
        self.decoration_instances.extend(
            decorations
                .iter()
                .map(|decoration| decoration.to_instance()),
        );
        self.request_ranges.clear();
        self.request_ranges.reserve(requests.len());

        let instance_build_started = profile.is_some().then(Instant::now);
        for (index, req) in requests.iter().enumerate() {
            // Record the glyph ranges this request will own. Both instance lists
            // are appended to in request order, so the slice for this request is
            // `[start, len_after)` in each list.
            let alpha_start = self.instances.len() as u32;
            let color_start = self.color_instances.len() as u32;

            // Off-screen text draws nothing this frame. Its range stays empty
            // so the ranges still line up with `requests`, which is what lets
            // a caller address one request's instances by index.
            if !visible[index] {
                self.request_ranges.push(TextRequestRange {
                    alpha_start,
                    alpha_end: alpha_start,
                    color_start,
                    color_end: color_start,
                });
                continue;
            }

            // Avoid cloning the span list on every frame (it ran even on a pure
            // cache hit). Borrow `req.spans` directly when present and only
            // allocate a one-element fallback when the request has no spans.
            let synthesized: [RichTextSpan; 1];
            let spans: &[RichTextSpan] = if req.spans.is_empty() {
                synthesized = [RichTextSpan::new(req.text.clone())];
                &synthesized
            } else {
                &req.spans
            };
            let span_key_range = self.visible_span_ranges[index].clone();

            let mut cursor_x = req.x;
            let mut cursor_y = req.y;

            for (span_index, span) in spans.iter().enumerate() {
                let key_index = span_key_range.start + span_index;
                debug_assert!(key_index < span_key_range.end);
                let color = span.color.unwrap_or(req.color);
                let font_weight = span
                    .font_weight
                    .or(req.font_weight)
                    .unwrap_or(FontWeight::Normal.numeric());
                // ponytail: synthetic (faux) italic via a horizontal shear in
                // the glyph shaders (0.25 ≈ 14°). Ceiling: not a real italic
                // face (no cursive glyph forms, advances unchanged). Upgrade
                // path: load a real italic/oblique face and key the atlas by it.
                let italic = span.italic.unwrap_or(req.italic);
                let paint_key = TextPaintCacheKey::new(
                    &self.visible_span_keys[key_index],
                    req,
                    font_weight,
                    italic,
                );
                let cached = if let Some(cached) = self.paint_cache.get(&paint_key) {
                    if let Some(profile) = profile.as_deref_mut() {
                        profile.paint_cache_hits += 1;
                    }
                    cached
                } else {
                    if let Some(profile) = profile.as_deref_mut() {
                        profile.paint_cache_misses += 1;
                    }
                    // Cloning is limited to misses so cache hits never hold
                    // an immutable borrow of the pipeline across template
                    // construction.
                    let keys = self.visible_span_keys[key_index].clone();
                    let Some(paint) = self.build_text_paint_template_generic(
                        backend,
                        req,
                        &keys,
                        font_weight,
                        italic,
                        profile.as_deref_mut(),
                    ) else {
                        return;
                    };
                    self.paint_cache.insert(paint_key, paint.clone());
                    paint
                };

                let span_coverage_exponent = coverage_exponent(color.to_unorm_array());
                let shadow = cached.shadow;
                let mut cull_key = None;
                let mut foreground_visible = false;
                let mut shadow_visible = false;
                for paint_glyph in &cached.glyphs {
                    if paint_glyph.kind == CachedPaintKind::Foreground && !req.draw_glyphs {
                        continue;
                    }

                    // The cached position is relative to the span origin. It
                    // must be snapped after the current request translation is
                    // added so scrolling preserves the original pixel-grid
                    // behavior without rebuilding the template.
                    let position = snap_to_pixel_grid([
                        paint_glyph.local_position[0] + cursor_x,
                        paint_glyph.local_position[1] + cursor_y,
                    ]);
                    let next_cull_key = (position, paint_glyph.size);
                    if cull_key != Some(next_cull_key) {
                        foreground_visible =
                            glyph_intersects_clip(position, paint_glyph.size, req.clip_rect);
                        shadow_visible = shadow.is_some_and(|shadow| {
                            shadow_intersects_clip(
                                position,
                                paint_glyph.size,
                                shadow,
                                req.clip_rect,
                            )
                        });
                        cull_key = Some(next_cull_key);
                    }
                    let visible = match paint_glyph.kind {
                        CachedPaintKind::Shadow => shadow_visible,
                        CachedPaintKind::Foreground => foreground_visible,
                    };
                    if !visible {
                        continue;
                    }

                    let (color, coverage_exponent) = match paint_glyph.kind {
                        CachedPaintKind::Shadow => {
                            let shadow = shadow.expect("cached shadow glyph requires a shadow");
                            (shadow.color, cached.shadow_coverage_exponent)
                        }
                        CachedPaintKind::Foreground => (color, span_coverage_exponent),
                    };
                    let instance = GlyphInstance {
                        position: [
                            position[0] + paint_glyph.offset[0],
                            position[1] + paint_glyph.offset[1],
                        ],
                        size: paint_glyph.size,
                        uv_rect: [
                            paint_glyph.region.x as f32,
                            paint_glyph.region.y as f32,
                            (paint_glyph.region.x + paint_glyph.region.width) as f32,
                            (paint_glyph.region.y + paint_glyph.region.height) as f32,
                        ],
                        color,
                        clip_rect: req.clip_rect,
                        clip_border_radius: req.clip_border_radius,
                        skew: paint_glyph.skew,
                        coverage_exponent,
                        _pad: [0.0; 2],
                    };

                    if paint_glyph.color_atlas {
                        self.color_instances.push(instance);
                    } else {
                        self.instances.push(instance);
                    }
                }

                cursor_x += cached.advance_x;
                cursor_y += cached.advance_y;
            }

            self.request_ranges.push(TextRequestRange {
                alpha_start,
                alpha_end: self.instances.len() as u32,
                color_start,
                color_end: self.color_instances.len() as u32,
            });
        }

        if let (Some(profile), Some(started)) = (profile.as_deref_mut(), instance_build_started) {
            profile.instance_build += started.elapsed();
        }

        // Now that every glyph has been inserted, the atlases have reached
        // their final dimensions for this frame. Resolve UVs against those
        // final dimensions so glyphs inserted before a mid-frame `grow()` are
        // not left referencing stale (smaller) atlas sizes.
        let (aw, ah) = (self.atlas.width, self.atlas.height);
        for instance in &mut self.instances {
            instance.uv_rect = normalize_pixel_uv_rect(instance.uv_rect, aw, ah);
        }
        let (cw, ch) = (self.color_atlas.width, self.color_atlas.height);
        for instance in &mut self.color_instances {
            instance.uv_rect = normalize_pixel_uv_rect(instance.uv_rect, cw, ch);
        }

        // Upload both atlases if new glyphs were added.
        let atlas_upload_started = profile.is_some().then(Instant::now);
        self.atlas.upload(backend);
        self.color_atlas.upload(backend);
        if let (Some(profile), Some(started)) = (profile.as_deref_mut(), atlas_upload_started) {
            profile.atlas_upload += started.elapsed();
        }

        // Rebuild bind groups only when their atlas texture was recreated (grow).
        let atlas_gen = self.atlas.generation();
        if atlas_gen != self.atlas_generation {
            self.atlas_generation = atlas_gen;
            self.bind_group = Self::create_bind_group(
                backend,
                &self.bind_group_layout,
                &self.viewport_buffer,
                &self.atlas.view,
                &self.sampler,
            );
        }
        let color_gen = self.color_atlas.generation();
        if color_gen != self.color_atlas_generation {
            self.color_atlas_generation = color_gen;
            self.color_bind_group = Self::create_bind_group(
                backend,
                &self.bind_group_layout,
                &self.viewport_buffer,
                &self.color_atlas.view,
                &self.sampler,
            );
        }
        #[cfg(target_os = "android")]
        let is_srgb_f32 = 2.0_f32;
        #[cfg(not(target_os = "android"))]
        let is_srgb_f32 = if is_srgb { 1.0_f32 } else { 0.0 };
        if self.last_viewport != (width, height) {
            self.last_viewport = (width, height);
            backend.write_buffer(
                &self.viewport_buffer,
                0,
                bytemuck::cast_slice(&[width as f32, height as f32, is_srgb_f32, 0.0]),
            );
        }

        let instance_upload_started = profile.is_some().then(Instant::now);
        let previous_capacity = self.instance_policy.capacity();
        self.instance_policy.record_usage(self.instances.len());
        if self.instance_policy.capacity() != previous_capacity {
            self.instance_buffer = backend.create_buffer(&crate::backend::BufferDescriptor {
                label: Some("text instance buffer".to_string()),
                size: (self.instance_policy.capacity() * size_of::<GlyphInstance>()) as u64,
                usage: vec![
                    crate::backend::BufferUsage::Vertex,
                    crate::backend::BufferUsage::CopyDst,
                ],
            });
            self.instance_upload.invalidate();
        }

        let previous_color_capacity = self.color_instance_policy.capacity();
        self.color_instance_policy
            .record_usage(self.color_instances.len());
        if self.color_instance_policy.capacity() != previous_color_capacity {
            self.color_instance_buffer = backend.create_buffer(&crate::backend::BufferDescriptor {
                label: Some("text color instance buffer".to_string()),
                size: (self.color_instance_policy.capacity() * size_of::<GlyphInstance>()) as u64,
                usage: vec![
                    crate::backend::BufferUsage::Vertex,
                    crate::backend::BufferUsage::CopyDst,
                ],
            });
            self.color_instance_upload.invalidate();
        }

        let previous_decoration_capacity = self.decoration_instance_policy.capacity();
        self.decoration_instance_policy
            .record_usage(self.decoration_instances.len());
        if self.decoration_instance_policy.capacity() != previous_decoration_capacity {
            self.decoration_instance_buffer =
                backend.create_buffer(&crate::backend::BufferDescriptor {
                    label: Some("text decoration instance buffer".to_string()),
                    size: (self.decoration_instance_policy.capacity()
                        * size_of::<DecorationInstance>()) as u64,
                    usage: vec![
                        crate::backend::BufferUsage::Vertex,
                        crate::backend::BufferUsage::CopyDst,
                    ],
                });
            self.decoration_instance_upload.invalidate();
        }

        // Upload the instance lists — each in one write, and only when their
        // bytes differ from what the GPU buffer already holds. Static text is
        // the common case, and it now costs no upload at all.
        self.instance_upload
            .upload(backend, &self.instance_buffer, &self.instances);
        self.color_instance_upload.upload(
            backend,
            &self.color_instance_buffer,
            &self.color_instances,
        );
        self.decoration_instance_upload.upload(
            backend,
            &self.decoration_instance_buffer,
            &self.decoration_instances,
        );
        if let (Some(profile), Some(started)) = (profile.as_deref_mut(), instance_upload_started) {
            profile.instance_upload += started.elapsed();
        }
        if self.postponed_preparation {
            // A partial ahead-of-view prepare is deliberately not cacheable:
            // the next identical frame must be allowed to continue the
            // deferred work rather than reusing only the visible prefix.
            self.prepared_frame = None;
        } else {
            self.prepared_frame = Some(PreparedFrameCache::new(
                width,
                height,
                is_srgb,
                self.atlas.generation(),
                self.color_atlas.generation(),
                FontRegistry::revision(),
                requests,
                decorations,
            ));
            self.prepared_frame_generation = self.frame_generation;
        }
        if let (Some(profile), Some(started)) = (profile.as_deref_mut(), profile_started) {
            profile.total = started.elapsed();
        }
    }
}
