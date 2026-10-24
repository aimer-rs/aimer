use std::ops::Range;
use std::rc::Rc;

use aimer_rubick::ShareRef;
use aimer_style::{
    FontFamily, FontStyle, FontWeight, TextDecoration, TextShadow, TextStyle, TextTransform,
};
use aimer_widget::base::Color;
use unicode_linebreak::linebreaks;
use unicode_segmentation::UnicodeSegmentation;

use crate::TextSource;

/// Optional run-level overrides for a [`TextSpan`].
///
/// Values left as `None` inherit from the parent span or the [`TextStyle`]
/// supplied to [`TextSpan::flatten`]. This keeps transformation, spacing,
/// decoration, and glyph-shadow inheritance explicit without duplicating the
/// base style on every span.
#[derive(Clone, Copy, Default, Debug)]
pub struct SpanStyle {
    /// Optional font-size override inherited from the parent span.
    pub font_size: Option<u32>,
    /// Optional font-family override inherited from the parent span.
    pub font_family: Option<FontFamily>,
    /// Optional font-style override inherited from the parent span.
    pub font_style: Option<FontStyle>,
    /// Optional font-weight override inherited from the parent span.
    pub font_weight: Option<FontWeight>,
    /// Optional foreground color override inherited from the parent span.
    pub color: Option<Color>,
    /// Optional inline background override inherited from the parent span.
    pub background_color: Option<Color>,
    /// Optional decoration override inherited from the parent span.
    pub text_decoration: Option<TextDecoration>,
    /// Optional Unicode transformation override inherited from the parent span.
    pub text_transform: Option<TextTransform>,
    /// Optional additional advance between adjacent rendered graphemes.
    pub letter_spacing: Option<f32>,
    /// Optional additional advance for whitespace word separators.
    pub word_spacing: Option<f32>,
    /// Optional glyph shadow override inherited from the parent span.
    pub text_shadow: Option<TextShadow>,
}

impl SpanStyle {
    pub const fn new() -> Self {
        Self {
            font_size: None,
            font_family: None,
            font_style: None,
            font_weight: None,
            color: None,
            background_color: None,
            text_decoration: None,
            text_transform: None,
            letter_spacing: None,
            word_spacing: None,
            text_shadow: None,
        }
    }

    pub const fn font_size(mut self, font_size: u32) -> Self {
        self.font_size = Some(font_size);
        self
    }

    pub const fn font_family(mut self, font_family: FontFamily) -> Self {
        self.font_family = Some(font_family);
        self
    }

    pub const fn font_style(mut self, font_style: FontStyle) -> Self {
        self.font_style = Some(font_style);
        self
    }

    pub const fn font_weight(mut self, font_weight: FontWeight) -> Self {
        self.font_weight = Some(font_weight);
        self
    }

    pub const fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    /// Overrides the inherited inline background without affecting layout.
    pub const fn background_color(mut self, background_color: Color) -> Self {
        self.background_color = Some(background_color);
        self
    }

    pub const fn text_decoration(mut self, text_decoration: TextDecoration) -> Self {
        self.text_decoration = Some(text_decoration);
        self
    }

    /// Overrides the inherited Unicode transformation for this span.
    #[inline]
    pub const fn text_transform(mut self, text_transform: TextTransform) -> Self {
        self.text_transform = Some(text_transform);
        self
    }

    /// Overrides the inherited additional advance between adjacent glyphs.
    #[inline]
    pub const fn letter_spacing(mut self, letter_spacing: f32) -> Self {
        self.letter_spacing = Some(letter_spacing);
        self
    }

    /// Overrides the inherited additional advance at word boundaries.
    #[inline]
    pub const fn word_spacing(mut self, word_spacing: f32) -> Self {
        self.word_spacing = Some(word_spacing);
        self
    }

    /// Adds a glyph shadow to this span, inheriting the base value otherwise.
    #[inline]
    pub const fn text_shadow(mut self, text_shadow: TextShadow) -> Self {
        self.text_shadow = Some(text_shadow);
        self
    }

    fn resolve(self, inherited: TextStyle) -> TextStyle {
        TextStyle {
            font_size: self.font_size.unwrap_or(inherited.font_size),
            font_family: self.font_family.unwrap_or(inherited.font_family),
            font_style: self.font_style.unwrap_or(inherited.font_style),
            font_weight: self.font_weight.unwrap_or(inherited.font_weight),
            color: self.color.unwrap_or(inherited.color),
            background_color: self.background_color.or(inherited.background_color),
            text_overflow: inherited.text_overflow,
            text_decoration: self.text_decoration.unwrap_or(inherited.text_decoration),
            text_transform: self.text_transform.unwrap_or(inherited.text_transform),
            letter_spacing: self.letter_spacing.unwrap_or(inherited.letter_spacing),
            word_spacing: self.word_spacing.unwrap_or(inherited.word_spacing),
            text_shadow: self.text_shadow.or(inherited.text_shadow),
        }
    }
}

#[derive(Clone)]
pub struct TextSpan {
    pub text: TextSource,
    pub style: SpanStyle,
    pub children: Vec<TextSpan>,
    pub link: Option<TextSource>,
}

impl TextSpan {
    #[inline]
    pub fn new(text: impl Into<Rc<str>>) -> Self {
        Self::from_text_source(TextSource::from(text.into()))
    }

    /// Creates a span that retains a shared string without copying its text.
    #[inline]
    pub fn new_shared(text: ShareRef<str>) -> Self {
        Self::from_text_source(TextSource::from(text))
    }

    fn from_text_source(text: TextSource) -> Self {
        Self {
            text,
            style: SpanStyle::new(),
            children: Vec::new(),
            link: None,
        }
    }

    #[inline]
    pub fn root(children: impl IntoIterator<Item = TextSpan>) -> Self {
        Self::new("").children(children)
    }

    #[inline]
    pub fn style(mut self, style: SpanStyle) -> Self {
        self.style = style;
        self
    }

    #[inline]
    pub fn children(mut self, children: impl IntoIterator<Item = TextSpan>) -> Self {
        self.children = children.into_iter().collect();
        self
    }

    #[inline]
    pub fn child(mut self, child: TextSpan) -> Self {
        self.children.push(child);
        self
    }

    #[inline]
    pub fn link(mut self, target: impl Into<Rc<str>>) -> Self {
        self.link = Some(TextSource::from(target.into()));
        self
    }

    /// Sets a link target while retaining its shared string handle.
    #[inline]
    pub fn link_shared(mut self, target: ShareRef<str>) -> Self {
        self.link = Some(TextSource::from(target));
        self
    }

    #[inline]
    pub fn flatten(&self, base_style: &TextStyle) -> Vec<ResolvedTextSpan> {
        let mut result = Vec::with_capacity(self.resolved_span_count());
        self.flatten_into(*base_style, None, &mut result);
        result
    }

    fn resolved_span_count(&self) -> usize {
        usize::from(!self.text.is_empty())
            + self
                .children
                .iter()
                .map(Self::resolved_span_count)
                .sum::<usize>()
    }

    fn flatten_into(
        &self,
        inherited_style: TextStyle,
        inherited_link: Option<TextSource>,
        result: &mut Vec<ResolvedTextSpan>,
    ) {
        let style = self.style.resolve(inherited_style);
        let link = self.link.clone().or(inherited_link);
        if !self.text.is_empty() {
            result.push(ResolvedTextSpan {
                text: self.text.clone(),
                style,
                link: link.clone(),
            });
        }
        for child in &self.children {
            child.flatten_into(style, link.clone(), result);
        }
    }
}

#[derive(Clone)]
pub struct ResolvedTextSpan {
    pub text: TextSource,
    pub style: TextStyle,
    pub link: Option<TextSource>,
}

impl ResolvedTextSpan {
    #[inline]
    pub fn plain(text: Rc<str>, style: TextStyle) -> Self {
        Self::plain_source(TextSource::from(text), style)
    }

    /// Creates a plain resolved span that retains a shared string handle.
    #[inline]
    pub fn plain_shared(text: ShareRef<str>, style: TextStyle) -> Self {
        Self::plain_source(TextSource::from(text), style)
    }

    /// Creates a plain resolved span without changing its text source.
    #[inline]
    pub fn plain_source(text: TextSource, style: TextStyle) -> Self {
        Self {
            text,
            style,
            link: None,
        }
    }
}

pub(crate) struct SpanLayout {
    pub fragments: Vec<SpanLayoutFragment>,
    pub line_breaks: Vec<SpanLayoutLineBreak>,
    pub line_count: usize,
}

pub(crate) struct SpanLayoutLineBreak {
    pub span_index: usize,
    pub source_range: Range<usize>,
    pub line: usize,
}

pub(crate) struct SpanLayoutFragment {
    pub span_index: usize,
    pub text: String,
    pub source_range: Option<Range<usize>>,
    pub rendered_source_ranges: Vec<Range<usize>>,
    pub line: usize,
    pub x: f32,
    pub width: f32,
}

fn transform_grapheme(
    grapheme: &str,
    transform: TextTransform,
    capitalize_next: &mut bool,
) -> String {
    match transform {
        TextTransform::None => grapheme.to_owned(),
        TextTransform::Uppercase => grapheme.chars().flat_map(char::to_uppercase).collect(),
        TextTransform::Lowercase => grapheme.chars().flat_map(char::to_lowercase).collect(),
        TextTransform::Capitalize => {
            let mut transformed = String::with_capacity(grapheme.len());
            for character in grapheme.chars() {
                if character.is_alphabetic() {
                    if *capitalize_next {
                        transformed.extend(character.to_uppercase());
                        *capitalize_next = false;
                    } else {
                        transformed.push(character);
                    }
                } else {
                    transformed.push(character);
                    if character.is_whitespace() || character.is_ascii_punctuation() {
                        *capitalize_next = true;
                    }
                }
            }
            transformed
        }
    }
}

fn rendered_source_ranges(text: &str, source_range: &Range<usize>) -> Vec<Range<usize>> {
    text.graphemes(true).map(|_| source_range.clone()).collect()
}

fn extend_source_range(source_range: &mut Option<Range<usize>>, appended_range: &Range<usize>) {
    if let Some(source_range) = source_range {
        source_range.end = appended_range.end;
    } else {
        *source_range = Some(appended_range.clone());
    }
}

fn source_range_from_rendered(rendered_source_ranges: &[Range<usize>]) -> Option<Range<usize>> {
    Some(rendered_source_ranges.first()?.start..rendered_source_ranges.last()?.end)
}

fn is_word_spacing_grapheme(grapheme: &str) -> bool {
    !grapheme.is_empty() && grapheme.chars().all(char::is_whitespace)
}

pub(crate) fn adjusted_grapheme_advance(
    grapheme: &str,
    style: &TextStyle,
    index: usize,
    count: usize,
    measure: &mut dyn FnMut(&str, &TextStyle) -> f32,
) -> f32 {
    let letter_spacing = if style.letter_spacing.is_finite() {
        style.letter_spacing
    } else {
        0.0
    };
    let word_spacing = if style.word_spacing.is_finite() {
        style.word_spacing
    } else {
        0.0
    };
    let spacing = if index + 1 < count {
        letter_spacing
    } else {
        0.0
    } + if is_word_spacing_grapheme(grapheme) {
        word_spacing
    } else {
        0.0
    };
    (measure(grapheme, style) + spacing).max(0.0)
}

pub(crate) fn adjusted_width(
    text: &str,
    style: &TextStyle,
    measure: &mut dyn FnMut(&str, &TextStyle) -> f32,
) -> f32 {
    let letter_spacing = if style.letter_spacing.is_finite() {
        style.letter_spacing
    } else {
        0.0
    };
    let word_spacing = if style.word_spacing.is_finite() {
        style.word_spacing
    } else {
        0.0
    };
    if letter_spacing == 0.0 && word_spacing == 0.0 {
        return measure(text, style);
    }

    let graphemes = text.graphemes(true).collect::<Vec<_>>();
    graphemes
        .iter()
        .enumerate()
        .map(|(index, grapheme)| {
            adjusted_grapheme_advance(grapheme, style, index, graphemes.len(), measure)
        })
        .sum::<f32>()
}

pub(crate) fn layout_resolved_spans(
    spans: &[ResolvedTextSpan],
    max_width: f32,
    measure: impl FnMut(&str, &TextStyle) -> f32,
) -> SpanLayout {
    layout_resolved_spans_with_indent(spans, max_width, 0.0, measure)
}

pub(crate) fn layout_resolved_spans_with_indent(
    spans: &[ResolvedTextSpan],
    max_width: f32,
    first_line_indent: f32,
    mut measure: impl FnMut(&str, &TextStyle) -> f32,
) -> SpanLayout {
    struct PendingGrapheme {
        span_index: usize,
        text: String,
        source_range: Range<usize>,
        rendered_source_ranges: Vec<Range<usize>>,
    }

    let plain_text = spans
        .iter()
        .map(|span| span.text.as_ref())
        .collect::<String>();
    let break_offsets = linebreaks(&plain_text)
        .map(|(offset, _)| offset)
        .collect::<Vec<_>>();
    let mut fragments = Vec::new();
    let mut line_breaks = Vec::new();
    let mut line = 0;
    let mut x = first_line_indent;
    let mut span_start = 0;
    let mut unit = Vec::new();
    let mut capitalize_next = true;
    let mut line_has_content = false;

    let place_unit = |unit: &mut Vec<PendingGrapheme>,
                      fragments: &mut Vec<SpanLayoutFragment>,
                      line: &mut usize,
                      x: &mut f32,
                      line_has_content: &mut bool,
                      measure: &mut dyn FnMut(&str, &TextStyle) -> f32| {
        if unit.is_empty() {
            return;
        }

        let mut runs: Vec<SpanLayoutFragment> = Vec::new();
        for grapheme in unit.iter() {
            if let Some(last) = runs.last_mut()
                && last.span_index == grapheme.span_index
            {
                last.text.push_str(&grapheme.text);
                last.rendered_source_ranges
                    .extend(grapheme.rendered_source_ranges.iter().cloned());
                extend_source_range(&mut last.source_range, &grapheme.source_range);
            } else {
                runs.push(SpanLayoutFragment {
                    span_index: grapheme.span_index,
                    text: grapheme.text.clone(),
                    source_range: Some(grapheme.source_range.clone()),
                    rendered_source_ranges: grapheme.rendered_source_ranges.clone(),
                    line: *line,
                    x: 0.0,
                    width: 0.0,
                });
            }
        }
        let unit_width = runs
            .iter_mut()
            .map(|run| {
                run.width = adjusted_width(&run.text, &spans[run.span_index].style, measure);
                run.width
            })
            .sum::<f32>();

        // Measuring separate words is fast, but shaping the complete painted run can
        // produce a slightly different width. Verify the complete line only
        // near its edge so wrapping and painting agree without reshaping the
        // growing line after every word.
        let mut verified_width = None;
        if max_width > 0.0
            && *line_has_content
            && *x + unit_width <= max_width
            && max_width - (*x + unit_width) <= unit_width
        {
            let mut line_runs: Vec<(usize, String)> = Vec::new();
            for fragment in fragments
                .iter()
                .filter(|fragment| fragment.line == *line)
                .chain(runs.iter())
            {
                if let Some((span_index, text)) = line_runs.last_mut()
                    && *span_index == fragment.span_index
                {
                    text.push_str(&fragment.text);
                } else {
                    line_runs.push((fragment.span_index, fragment.text.clone()));
                }
            }
            verified_width = Some(
                line_runs
                    .iter()
                    .map(|(span_index, text)| {
                        adjusted_width(text, &spans[*span_index].style, measure)
                    })
                    .sum::<f32>(),
            );
        }

        if max_width > 0.0
            && *line_has_content
            && (*x + unit_width > max_width
                || verified_width.is_some_and(|width| width > max_width))
        {
            *line += 1;
            *x = 0.0;
            *line_has_content = false;
            verified_width = None;
        }

        if max_width <= 0.0 || unit_width <= max_width {
            for mut run in runs {
                run.line = *line;
                run.x = *x;
                *x += run.width;
                *line_has_content = true;
                fragments.push(run);
            }
            if let Some(width) = verified_width {
                *x = width;
            }
            unit.clear();
            return;
        }

        // A single word can be wider than the line. Grow exact shaped chunks until the
        // next grapheme would overflow, then continue that word on the
        // following line.
        let mut chunk: Option<SpanLayoutFragment> = None;
        for grapheme in unit.drain(..) {
            let same_span = chunk
                .as_ref()
                .is_some_and(|fragment| fragment.span_index == grapheme.span_index);
            let mut candidate = if same_span {
                let mut candidate = chunk.take().unwrap();
                candidate.text.push_str(&grapheme.text);
                candidate
                    .rendered_source_ranges
                    .extend(grapheme.rendered_source_ranges.iter().cloned());
                extend_source_range(&mut candidate.source_range, &grapheme.source_range);
                candidate
            } else {
                if let Some(fragment) = chunk.take() {
                    *x += fragment.width;
                    *line_has_content = true;
                    fragments.push(fragment);
                }
                SpanLayoutFragment {
                    span_index: grapheme.span_index,
                    text: grapheme.text.clone(),
                    source_range: Some(grapheme.source_range.clone()),
                    rendered_source_ranges: grapheme.rendered_source_ranges.clone(),
                    line: *line,
                    x: *x,
                    width: 0.0,
                }
            };
            candidate.width =
                adjusted_width(&candidate.text, &spans[candidate.span_index].style, measure);

            if !same_span
                && max_width > 0.0
                && *line_has_content
                && *x + candidate.width > max_width
            {
                *line += 1;
                *x = 0.0;
                *line_has_content = false;
                candidate.line = *line;
                candidate.x = 0.0;
            }

            if max_width > 0.0
                && candidate.text != grapheme.text
                && *x + candidate.width > max_width
            {
                let split_at = candidate.text.len() - grapheme.text.len();
                candidate.text.truncate(split_at);
                candidate.rendered_source_ranges.truncate(
                    candidate.rendered_source_ranges.len() - grapheme.rendered_source_ranges.len(),
                );
                candidate.source_range =
                    source_range_from_rendered(&candidate.rendered_source_ranges);
                candidate.width =
                    adjusted_width(&candidate.text, &spans[candidate.span_index].style, measure);
                fragments.push(candidate);
                *line += 1;
                *x = 0.0;
                *line_has_content = false;
                chunk = Some(SpanLayoutFragment {
                    span_index: grapheme.span_index,
                    text: grapheme.text.clone(),
                    source_range: Some(grapheme.source_range),
                    rendered_source_ranges: grapheme.rendered_source_ranges.clone(),
                    line: *line,
                    x: 0.0,
                    width: adjusted_width(
                        &grapheme.text,
                        &spans[grapheme.span_index].style,
                        measure,
                    ),
                });
            } else {
                candidate.line = *line;
                candidate.x = *x;
                chunk = Some(candidate);
            }
        }
        if let Some(fragment) = chunk {
            *x += fragment.width;
            *line_has_content = true;
            fragments.push(fragment);
        }
    };

    for (span_index, span) in spans.iter().enumerate() {
        for (grapheme_start, grapheme) in span.text.grapheme_indices(true) {
            let source_range =
                span_start + grapheme_start..span_start + grapheme_start + grapheme.len();
            if grapheme == "\n" || grapheme == "\r\n" {
                place_unit(
                    &mut unit,
                    &mut fragments,
                    &mut line,
                    &mut x,
                    &mut line_has_content,
                    &mut measure,
                );
                line_breaks.push(SpanLayoutLineBreak {
                    span_index,
                    source_range,
                    line,
                });
                line += 1;
                x = 0.0;
                line_has_content = false;
                continue;
            }

            let is_break = break_offsets.binary_search(&source_range.end).is_ok();
            let transformed =
                transform_grapheme(grapheme, span.style.text_transform, &mut capitalize_next);
            let rendered_source_ranges = if span.style.text_transform == TextTransform::None {
                Vec::new()
            } else {
                rendered_source_ranges(&transformed, &source_range)
            };
            unit.push(PendingGrapheme {
                span_index,
                rendered_source_ranges,
                text: transformed,
                source_range,
            });
            if is_break {
                place_unit(
                    &mut unit,
                    &mut fragments,
                    &mut line,
                    &mut x,
                    &mut line_has_content,
                    &mut measure,
                );
            }
        }
        span_start += span.text.len();
    }
    place_unit(
        &mut unit,
        &mut fragments,
        &mut line,
        &mut x,
        &mut line_has_content,
        &mut measure,
    );

    let mut merged: Vec<SpanLayoutFragment> = Vec::new();
    for fragment in fragments {
        if let Some(previous) = merged.last_mut()
            && previous.span_index == fragment.span_index
            && previous.line == fragment.line
        {
            previous.text.push_str(&fragment.text);
            previous
                .rendered_source_ranges
                .extend(fragment.rendered_source_ranges);
            if let Some(source_range) = fragment.source_range.as_ref() {
                extend_source_range(&mut previous.source_range, source_range);
            }
        } else {
            merged.push(fragment);
        }
    }

    let mut measured_line = usize::MAX;
    let mut measured_x = first_line_indent;
    for fragment in &mut merged {
        if fragment.line != measured_line {
            measured_line = fragment.line;
            measured_x = if measured_line == 0 {
                first_line_indent
            } else {
                0.0
            };
        }
        fragment.x = measured_x;
        fragment.width = adjusted_width(
            &fragment.text,
            &spans[fragment.span_index].style,
            &mut measure,
        );
        measured_x += fragment.width;
    }

    SpanLayout {
        fragments: merged,
        line_breaks,
        line_count: line + 1,
    }
}

pub(crate) fn ellipsize_first_line(
    layout: &mut SpanLayout,
    spans: &[ResolvedTextSpan],
    max_width: f32,
    mut measure: impl FnMut(&str, &TextStyle) -> f32,
) {
    if layout.line_count <= 1 || spans.is_empty() {
        return;
    }

    layout.fragments.retain(|fragment| fragment.line == 0);
    layout.line_breaks.clear();
    let span_index = layout
        .fragments
        .last()
        .map(|fragment| fragment.span_index)
        .unwrap_or(0);
    loop {
        let too_wide = layout.fragments.last().is_some_and(|fragment| {
            let mut text = fragment.text.clone();
            text.push('…');
            fragment.x + adjusted_width(&text, &spans[fragment.span_index].style, &mut measure)
                > max_width
        });
        if !too_wide {
            break;
        }
        let last = layout.fragments.last_mut().expect("a fragment exists");
        if let Some(start) = last
            .text
            .grapheme_indices(true)
            .next_back()
            .map(|(start, _)| start)
        {
            last.text.truncate(start);
            if last.rendered_source_ranges.is_empty() {
                if let Some(source_range) = last.source_range.as_mut() {
                    source_range.end = source_range.start + last.text.len();
                }
            } else {
                last.rendered_source_ranges.pop();
                last.source_range = source_range_from_rendered(&last.rendered_source_ranges);
            }
            last.width = adjusted_width(&last.text, &spans[last.span_index].style, &mut measure);
        }
        if last.text.is_empty() {
            layout.fragments.pop();
        }
    }

    if let Some(last) = layout.fragments.last_mut() {
        last.text.push('…');
        let mut rendered_text = last.text.clone();
        rendered_text.pop();
        rendered_text.push('…');
        last.width = adjusted_width(&rendered_text, &spans[last.span_index].style, &mut measure);
    } else {
        let ellipsis_width = adjusted_width("…", &spans[span_index].style, &mut measure);
        layout.fragments.push(SpanLayoutFragment {
            span_index,
            text: "…".to_owned(),
            source_range: None,
            rendered_source_ranges: Vec::new(),
            line: 0,
            x: 0.0,
            width: ellipsis_width,
        });
    }
    layout.line_count = 1;
}

#[cfg(test)]
mod tests {
    use aimer_rubick::ShareRef;
    use aimer_style::{FontFamily, FontWeight, TextShadow, TextStyle, TextTransform};
    use aimer_widget::base::Color;

    use super::*;
    use crate::TextSource;

    #[test]
    fn plain_source_keeps_share_ref_backing() {
        let source = ShareRef::from_static("shared plain span text");
        let span =
            ResolvedTextSpan::plain_source(TextSource::from(source.clone()), TextStyle::default());
        let TextSource::ShareRef(span_source) = span.text else {
            panic!("plain resolved span should retain its shared source")
        };

        assert!(ShareRef::ptr_eq(&source, &span_source));
    }

    #[test]
    fn flatten_preserves_shared_text_and_link_storage() {
        let text = ShareRef::from_static("shared span text");
        let link = ShareRef::from_static("https://aimer.dev");
        let span = TextSpan::new_shared(text.clone()).link_shared(link.clone());

        let TextSource::ShareRef(span_text) = &span.text else {
            panic!("ShareRef input should remain shared in the span")
        };
        assert!(ShareRef::ptr_eq(&text, span_text));

        let flattened = span.flatten(&TextStyle::default());
        assert_eq!(flattened.len(), 1);
        let TextSource::ShareRef(flattened_text) = &flattened[0].text else {
            panic!("flattening should preserve the shared text source")
        };
        let Some(TextSource::ShareRef(flattened_link)) = &flattened[0].link else {
            panic!("flattening should preserve the shared link source")
        };
        assert!(ShareRef::ptr_eq(&text, flattened_text));
        assert!(ShareRef::ptr_eq(&link, flattened_link));
    }

    #[test]
    fn nested_spans_inherit_and_override_parent_style() {
        let root = TextSpan::new("prefix ")
            .style(
                SpanStyle::new()
                    .font_weight(FontWeight::Bold)
                    .color(Color::RED),
            )
            .children([
                TextSpan::new("inherited"),
                TextSpan::new(" overridden").style(SpanStyle::new().color(Color::BLUE)),
            ]);

        let flattened = root.flatten(&TextStyle::new().font_size(18));

        assert_eq!(flattened.len(), 3);
        assert_eq!(&*flattened[1].text, "inherited");
        assert_eq!(flattened[1].style.font_size, 18);
        assert_eq!(
            flattened[1].style.font_weight.numeric(),
            FontWeight::Bold.numeric()
        );
        assert_eq!(flattened[1].style.color, Color::RED);
        assert_eq!(flattened[2].style.color, Color::BLUE);
    }

    #[test]
    fn nested_spans_inherit_and_override_font_family() {
        let custom = FontFamily::MONOSPACE;
        let flattened = TextSpan::new("parent")
            .style(SpanStyle::new().font_family(custom))
            .children([
                TextSpan::new(" inherited"),
                TextSpan::new(" sans").style(SpanStyle::new().font_family(FontFamily::SANS_SERIF)),
            ])
            .flatten(&TextStyle::default());

        assert_eq!(flattened[0].style.font_family, custom);
        assert_eq!(flattened[1].style.font_family, custom);
        assert_eq!(flattened[2].style.font_family, FontFamily::SANS_SERIF);
    }

    #[test]
    fn nested_spans_inherit_and_override_background_color() {
        let flattened = TextSpan::new("parent")
            .style(SpanStyle::new().background_color(Color::RED))
            .children([
                TextSpan::new(" inherited"),
                TextSpan::new(" blue").style(SpanStyle::new().background_color(Color::BLUE)),
            ])
            .flatten(&TextStyle::default());

        assert_eq!(flattened[0].style.background_color, Some(Color::RED));
        assert_eq!(flattened[1].style.background_color, Some(Color::RED));
        assert_eq!(flattened[2].style.background_color, Some(Color::BLUE));
        assert_eq!(TextStyle::default().background_color, None);
    }

    #[test]
    fn span_style_inherits_and_overrides_text_run_properties() {
        let shadow = TextShadow::new().offset_x(2.0);
        let root = TextSpan::new("child").style(
            SpanStyle::new()
                .text_transform(TextTransform::Uppercase)
                .letter_spacing(0.5)
                .word_spacing(1.0)
                .text_shadow(shadow),
        );

        let flattened = root.flatten(
            &TextStyle::new()
                .text_transform(TextTransform::Lowercase)
                .letter_spacing(-0.25)
                .word_spacing(-0.5),
        );

        assert_eq!(flattened[0].style.text_transform, TextTransform::Uppercase);
        assert_eq!(flattened[0].style.letter_spacing, 0.5);
        assert_eq!(flattened[0].style.word_spacing, 1.0);
        assert_eq!(flattened[0].style.text_shadow, Some(shadow));
    }

    #[test]
    fn uppercase_expansion_keeps_each_rendered_cluster_on_its_source_range() {
        let spans = vec![ResolvedTextSpan::plain(
            Rc::from("ß"),
            TextStyle::new().text_transform(TextTransform::Uppercase),
        )];

        let layout =
            layout_resolved_spans(&spans, 0.0, |text, _| text.graphemes(true).count() as f32);

        assert_eq!(layout.fragments[0].text, "SS");
        assert_eq!(layout.fragments[0].rendered_source_ranges, vec![0..2, 0..2]);
    }

    #[test]
    fn untransformed_spans_use_direct_source_offsets() {
        let layout = layout_resolved_spans(
            &[ResolvedTextSpan::plain(
                Rc::from("ordinary rich text"),
                TextStyle::new(),
            )],
            0.0,
            |text, _| text.len() as f32,
        );

        assert!(layout.fragments[0].rendered_source_ranges.is_empty());
    }

    #[test]
    fn lowercase_and_capitalize_transform_unicode_without_losing_graphemes() {
        let lowercase = layout_resolved_spans(
            &[ResolvedTextSpan::plain(
                Rc::from("ÄBC e\u{301}"),
                TextStyle::new().text_transform(TextTransform::Lowercase),
            )],
            0.0,
            |text, _| text.graphemes(true).count() as f32,
        );
        assert_eq!(lowercase.fragments[0].text, "äbc e\u{301}");

        let capitalized = layout_resolved_spans(
            &[ResolvedTextSpan::plain(
                Rc::from("hello, world! nächste"),
                TextStyle::new().text_transform(TextTransform::Capitalize),
            )],
            0.0,
            |text, _| text.graphemes(true).count() as f32,
        );
        assert_eq!(capitalized.fragments[0].text, "Hello, World! Nächste");
        assert_eq!(
            capitalized.fragments[0].rendered_source_ranges.len(),
            capitalized.fragments[0].text.graphemes(true).count()
        );
    }

    #[test]
    fn spacing_handles_empty_text_whitespace_combining_marks_and_mixed_spans() {
        let empty = layout_resolved_spans(
            &[ResolvedTextSpan::plain(
                Rc::from(""),
                TextStyle::new().letter_spacing(4.0).word_spacing(6.0),
            )],
            0.0,
            |text, _| text.len() as f32,
        );
        assert!(empty.fragments.is_empty());

        let whitespace = layout_resolved_spans(
            &[ResolvedTextSpan::plain(
                Rc::from("  "),
                TextStyle::new().word_spacing(2.0),
            )],
            0.0,
            |text, _| text.graphemes(true).count() as f32,
        );
        assert_eq!(whitespace.fragments[0].width, 6.0);

        let combining = layout_resolved_spans(
            &[ResolvedTextSpan::plain(
                Rc::from("e\u{301}x"),
                TextStyle::new().letter_spacing(1.0),
            )],
            0.0,
            |text, _| text.graphemes(true).count() as f32,
        );
        assert_eq!(combining.fragments[0].width, 3.0);

        let mixed = layout_resolved_spans(
            &[
                ResolvedTextSpan::plain(
                    Rc::from("AB"),
                    TextStyle::new()
                        .text_transform(TextTransform::Lowercase)
                        .letter_spacing(1.0),
                ),
                ResolvedTextSpan::plain(Rc::from(" cd"), TextStyle::new().word_spacing(2.0)),
            ],
            0.0,
            |text, _| text.graphemes(true).count() as f32,
        );
        assert_eq!(mixed.fragments[0].text, "ab");
        assert_eq!(mixed.fragments[0].width, 3.0);
        assert_eq!(mixed.fragments[1].text, " cd");
        assert_eq!(mixed.fragments[1].width, 5.0);
    }

    #[test]
    fn spacing_participates_in_wrapping_instead_of_paint_only_offsets() {
        let spans = vec![ResolvedTextSpan::plain(
            Rc::from("ab cd"),
            TextStyle::new().letter_spacing(1.0),
        )];

        let layout =
            layout_resolved_spans(&spans, 5.0, |text, _| text.graphemes(true).count() as f32);

        assert_eq!(layout.line_count, 2);
        assert_eq!(layout.fragments[0].text, "ab ");
        assert_eq!(layout.fragments[0].width, 5.0);
        assert_eq!(layout.fragments[1].text, "cd");
    }

    #[test]
    fn spacing_is_part_of_the_measured_run_width() {
        let spans = vec![ResolvedTextSpan::plain(
            Rc::from("a b"),
            TextStyle::new().letter_spacing(1.0).word_spacing(2.0),
        )];

        let layout =
            layout_resolved_spans(&spans, 0.0, |text, _| text.graphemes(true).count() as f32);

        assert_eq!(layout.fragments[0].width, 7.0);
    }

    #[test]
    fn first_line_indent_changes_only_the_first_line_origin() {
        let spans = vec![ResolvedTextSpan::plain(
            Rc::from("one two"),
            TextStyle::default(),
        )];

        let layout = layout_resolved_spans_with_indent(&spans, 5.0, 2.0, |text, _| {
            text.graphemes(true).count() as f32
        });

        assert_eq!(layout.fragments[0].x, 2.0);
        assert!(layout.fragments.iter().any(|fragment| fragment.line > 0));
        assert!(
            layout
                .fragments
                .iter()
                .filter(|fragment| fragment.line > 0)
                .all(|fragment| fragment.x == 0.0)
        );
    }

    #[test]
    fn negative_first_line_indent_is_a_hanging_indent() {
        let spans = vec![ResolvedTextSpan::plain(
            Rc::from("one two three"),
            TextStyle::default(),
        )];

        let layout = layout_resolved_spans_with_indent(&spans, 5.0, -2.0, |text, _| {
            text.graphemes(true).count() as f32
        });

        assert_eq!(layout.fragments[0].x, -2.0);
        assert!(layout.fragments.iter().any(|fragment| fragment.line > 0));
        assert!(
            layout
                .fragments
                .iter()
                .filter(|fragment| fragment.line > 0)
                .all(|fragment| fragment.x == 0.0)
        );
    }

    #[test]
    fn link_target_is_inherited_by_nested_text() {
        let root = TextSpan::root([TextSpan::new("").link("https://aimer.dev").children([
            TextSpan::new("Aimer "),
            TextSpan::new("docs").style(SpanStyle::new().font_weight(FontWeight::Bold)),
        ])]);

        let flattened = root.flatten(&TextStyle::default());

        assert_eq!(flattened.len(), 2);
        assert!(
            flattened
                .iter()
                .all(|span| span.link.as_deref() == Some("https://aimer.dev"))
        );
    }

    #[test]
    fn flatten_reserves_exactly_the_number_of_resolved_spans() {
        let root = TextSpan::root([
            TextSpan::new("").children([
                TextSpan::new("one"),
                TextSpan::new("two"),
                TextSpan::new("three"),
            ]),
            TextSpan::new("four"),
            TextSpan::new("five"),
        ]);

        let flattened = root.flatten(&TextStyle::default());

        assert_eq!(flattened.len(), 5);
        assert_eq!(flattened.capacity(), flattened.len());
    }

    #[test]
    fn layout_fragments_retain_global_unicode_source_ranges_across_spans() {
        let spans = vec![
            ResolvedTextSpan::plain(Rc::from("aé"), TextStyle::default()),
            ResolvedTextSpan::plain(Rc::from("👩‍💻b"), TextStyle::default()),
        ];

        let layout =
            layout_resolved_spans(&spans, 2.0, |text, _| text.graphemes(true).count() as f32);

        assert_eq!(layout.fragments.len(), 2);
        assert_eq!(layout.fragments[0].text, "aé");
        assert_eq!(layout.fragments[0].source_range, Some(0..3));
        assert_eq!(layout.fragments[1].text, "👩‍💻b");
        assert_eq!(layout.fragments[1].source_range, Some(3..15));
    }

    #[test]
    fn source_ranges_include_explicit_newlines_between_visible_fragments() {
        let spans = vec![ResolvedTextSpan::plain(
            Rc::from("first\nsecond"),
            TextStyle::default(),
        )];

        let layout = layout_resolved_spans(&spans, 0.0, |text, _| text.len() as f32);

        assert_eq!(layout.fragments.len(), 2);
        assert_eq!(layout.fragments[0].source_range, Some(0..5));
        assert_eq!(layout.fragments[1].source_range, Some(6..12));
    }

    #[test]
    fn explicit_line_breaks_retain_their_source_ranges_and_styles() {
        let spans = vec![
            ResolvedTextSpan::plain(Rc::from("first\n"), TextStyle::new().font_size(14)),
            ResolvedTextSpan::plain(Rc::from("\nsecond"), TextStyle::new().font_size(20)),
        ];

        let layout = layout_resolved_spans(&spans, 0.0, |text, _| text.len() as f32);

        assert_eq!(layout.line_count, 3);
        assert_eq!(layout.line_breaks.len(), 2);
        assert_eq!(layout.line_breaks[0].span_index, 0);
        assert_eq!(layout.line_breaks[0].source_range, 5..6);
        assert_eq!(layout.line_breaks[0].line, 0);
        assert_eq!(layout.line_breaks[1].span_index, 1);
        assert_eq!(layout.line_breaks[1].source_range, 6..7);
        assert_eq!(layout.line_breaks[1].line, 1);
    }

    #[test]
    fn wrapping_prefers_word_boundaries() {
        let spans = vec![ResolvedTextSpan::plain(
            Rc::from("hello world"),
            TextStyle::default(),
        )];

        let layout = layout_resolved_spans(&spans, 8.0, |text, _| text.chars().count() as f32);

        assert_eq!(layout.line_count, 2);
        assert_eq!(layout.fragments[0].text, "hello ");
        assert_eq!(layout.fragments[0].line, 0);
        assert_eq!(layout.fragments[1].text, "world");
        assert_eq!(layout.fragments[1].line, 1);
    }

    #[test]
    fn an_overlong_word_falls_back_to_grapheme_wrapping() {
        let spans = vec![ResolvedTextSpan::plain(
            Rc::from("abcdefgh"),
            TextStyle::default(),
        )];

        let layout = layout_resolved_spans(&spans, 3.0, |text, _| text.chars().count() as f32);

        assert_eq!(layout.line_count, 3);
        assert_eq!(layout.fragments[0].text, "abc");
        assert_eq!(layout.fragments[1].text, "def");
        assert_eq!(layout.fragments[2].text, "gh");
    }

    #[test]
    fn overlong_word_wraps_when_its_style_changes_at_the_line_edge() {
        let spans = vec![
            ResolvedTextSpan::plain(Rc::from("abc"), TextStyle::default()),
            ResolvedTextSpan::plain(Rc::from("d"), TextStyle::new().font_size(18)),
        ];

        let layout = layout_resolved_spans(&spans, 3.0, |text, _| text.len() as f32);

        assert_eq!(layout.line_count, 2);
        assert_eq!(layout.fragments[0].text, "abc");
        assert_eq!(layout.fragments[0].line, 0);
        assert_eq!(layout.fragments[1].text, "d");
        assert_eq!(layout.fragments[1].line, 1);
    }

    #[test]
    fn word_wrapping_continues_across_style_span_boundaries() {
        let spans = vec![
            ResolvedTextSpan::plain(Rc::from("hel"), TextStyle::default()),
            ResolvedTextSpan::plain(Rc::from("lo world"), TextStyle::new().font_size(18)),
        ];

        let layout = layout_resolved_spans(&spans, 7.0, |text, _| text.chars().count() as f32);
        let first_line = layout
            .fragments
            .iter()
            .filter(|fragment| fragment.line == 0)
            .map(|fragment| fragment.text.as_str())
            .collect::<String>();
        let second_line = layout
            .fragments
            .iter()
            .filter(|fragment| fragment.line == 1)
            .map(|fragment| fragment.text.as_str())
            .collect::<String>();

        assert_eq!(layout.line_count, 2);
        assert_eq!(first_line, "hello ");
        assert_eq!(second_line, "world");
    }

    #[test]
    fn wrapping_uses_the_shaped_width_of_complete_runs() {
        let spans = vec![ResolvedTextSpan::plain(
            Rc::from("AV"),
            TextStyle::default(),
        )];

        let layout = layout_resolved_spans(&spans, 10.0, |text, _| match text {
            "AV" => 10.0,
            "A" | "V" => 6.0,
            _ => unreachable!("unexpected measurement: {text}"),
        });

        assert_eq!(layout.line_count, 1);
        assert_eq!(layout.fragments[0].text, "AV");
        assert_eq!(layout.fragments[0].width, 10.0);
    }

    #[test]
    fn wrapping_accounts_for_reshaping_adjacent_words() {
        let spans = vec![ResolvedTextSpan::plain(
            Rc::from("one two"),
            TextStyle::default(),
        )];

        let layout = layout_resolved_spans(&spans, 7.0, |text, _| match text {
            "one " => 4.0,
            "two" => 3.0,
            "one two" => 8.0,
            _ => text.len() as f32,
        });

        assert_eq!(layout.line_count, 2);
        assert_eq!(layout.fragments[0].text, "one ");
        assert_eq!(layout.fragments[1].text, "two");
        assert!(
            layout
                .fragments
                .iter()
                .all(|fragment| fragment.x + fragment.width <= 7.0)
        );
    }

    #[test]
    fn unwrapped_words_are_measured_as_runs_instead_of_every_grapheme() {
        let spans = vec![ResolvedTextSpan::plain(
            Rc::from("Rich text resizing should stay responsive"),
            TextStyle::default(),
        )];
        let mut measurements = 0;

        let layout = layout_resolved_spans(&spans, 1_000.0, |text, _| {
            measurements += 1;
            text.len() as f32
        });

        assert_eq!(layout.line_count, 1);
        assert!(
            measurements <= 7,
            "layout performed {measurements} shaping measurements"
        );
    }

    #[test]
    fn ellipsis_keeps_one_line_and_fits_the_available_width() {
        let style = TextStyle::new().font_size(10);
        let spans = vec![ResolvedTextSpan::plain(Rc::from("abcdef"), style)];
        let mut layout =
            layout_resolved_spans(&spans, 20.0, |text, _| text.chars().count() as f32 * 5.0);

        ellipsize_first_line(&mut layout, &spans, 20.0, |text, _| {
            text.chars().count() as f32 * 5.0
        });

        assert_eq!(layout.line_count, 1);
        assert_eq!(
            layout
                .fragments
                .iter()
                .map(|fragment| fragment.text.as_str())
                .collect::<String>(),
            "abc…"
        );
        assert!(
            layout
                .fragments
                .iter()
                .map(|fragment| fragment.width)
                .sum::<f32>()
                <= 20.0
        );
    }
}
