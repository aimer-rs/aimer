use aimer_attribute::position::Vec2d;
use aimer_cupid::canvas::CupidCanvas;
use aimer_cupid::font::{FontFamily, FontStyle, TextLanguage};
use aimer_cupid::utilities::Mat3;

use crate::canvas::CanvasRendering;

#[allow(dead_code)]
impl CanvasRendering for CupidCanvas {
    #[inline]
    fn begin_frame(&self) {
        CupidCanvas::begin_frame(self);
    }

    #[inline]
    fn translate(&self, pos: Vec2d) {
        CupidCanvas::translate(self, pos.x, pos.y);
    }

    #[inline]
    fn scale(&self, sx: f32, sy: f32) {
        CupidCanvas::scale(self, sx, sy);
    }

    #[inline]
    fn rotate(&self, radians: f32) {
        CupidCanvas::rotate(self, radians);
    }

    #[inline]
    fn save(&self) {
        CupidCanvas::save(self);
    }

    #[inline]
    fn restore(&self) {
        CupidCanvas::restore(self);
    }

    #[inline]
    fn get_image_size(&self, image_id: u32) -> Option<(u32, u32)> {
        CupidCanvas::get_image_size(self, image_id)
    }

    #[inline]
    fn texture_cache_epoch(&self) -> u64 {
        CupidCanvas::texture_cache_epoch(self)
    }

    #[inline]
    fn is_texture_available(&self, image_id: u32) -> bool {
        CupidCanvas::is_texture_available(self, image_id)
    }

    #[inline]
    fn measure_text(&self, text: &str, font_size: f32) -> f32 {
        CupidCanvas::measure_text(self, text, font_size)
    }

    #[inline]
    fn measure_text_styled(
        &self,
        text: &str,
        font_size: f32,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
    ) -> f32 {
        CupidCanvas::measure_text_styled(
            self,
            text,
            font_size,
            font_family,
            font_style,
            font_weight,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn layout_text_styled(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
    ) -> crate::canvas::TextInteractionLayout {
        CupidCanvas::layout_text_styled(
            self,
            text,
            font_size,
            max_width,
            font_family,
            font_style,
            font_weight,
        )
    }

    #[inline]
    fn measure_text_metrics(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
    ) -> crate::canvas::TextMetrics {
        CupidCanvas::measure_text_metrics(self, text, font_size, max_width)
    }

    #[inline]
    fn measure_text_metrics_styled(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
    ) -> crate::canvas::TextMetrics {
        CupidCanvas::measure_text_metrics_styled(
            self,
            text,
            font_size,
            max_width,
            font_family,
            font_style,
            font_weight,
        )
    }

    #[inline]
    fn measure_text_line_widths_styled(
        &self,
        text: &str,
        font_size: f32,
        max_width: f32,
        font_family: FontFamily,
        font_style: FontStyle,
        font_weight: u16,
    ) -> Vec<f32> {
        CupidCanvas::measure_text_line_widths_styled(
            self,
            text,
            font_size,
            max_width,
            font_family,
            font_style,
            font_weight,
        )
    }

    #[inline]
    fn set_text_language(&self, language: Option<TextLanguage>) {
        CupidCanvas::set_text_language(self, language);
    }

    #[inline]
    fn text_language(&self) -> Option<TextLanguage> {
        CupidCanvas::text_language(self)
    }

    #[inline]
    fn load_image(&self, bytes: &[u8], width: u32, height: u32) -> u32 {
        self.load_image(bytes, width, height)
    }

    fn load_image_with_id(&self, image_id: u32, bytes: &[u8], width: u32, height: u32) {
        self.load_image_with_id(image_id, bytes, width, height)
    }

    fn remove_texture(&self, image_id: u32) {
        self.remove_texture(image_id)
    }

    #[inline]
    fn set_texture_size(&self, image_id: u32, width: u32, height: u32) {
        self.set_texture_size(image_id, width, height)
    }

    #[inline]
    fn get_transform_translation(&self) -> (f32, f32) {
        let (tx, ty) = CupidCanvas::get_transform_translation(self);
        (tx, ty)
    }

    #[inline]
    fn get_transform(&self) -> Mat3 {
        CupidCanvas::get_transform(self)
    }
}
