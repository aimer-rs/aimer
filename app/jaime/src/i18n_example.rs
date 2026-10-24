//! Jaime's deterministic localization and directionality example.
//!
//! W17 registers this page in the shared showcase and exposes the
//! `aimer_i18n` model through the umbrella crate.

use aimer::i18n::{
    DateFormatStyle, DateTimeFormatOptions, Locale, MissingTranslationPolicy, NumberFormatOptions,
    TimeFormatStyle, TranslationCatalog, Translator, UtcOffset, format_datetime, format_number,
};
use aimer::{AnyElement, BuildContext, Column, Container, Text, Widget};

/// A small localization page showing translated, pluralized, formatted, and
/// direction-aware values.
pub struct I18nExample {
    locale: Locale,
    greeting: String,
    item_count: String,
    formatted_number: String,
    formatted_datetime: String,
    direction: String,
    validation_message: String,
    accessibility_label: String,
}

impl I18nExample {
    /// Creates a deterministic French example using a fixed UTC offset.
    pub fn new() -> Self {
        let locale = Locale::parse("fr-FR").expect("the example locale is valid");
        let mut catalog = TranslationCatalog::new();
        catalog.add_translations(
            Locale::parse("fr").expect("the fallback locale is valid"),
            [
                ("greeting", "Bonjour"),
                ("cart.items.one", "{count} article"),
                ("cart.items.other", "{count} articles"),
                ("validation.required", "Ce champ est obligatoire"),
                ("accessibility.close", "Fermer"),
            ],
        );
        let translator = Translator::from_catalog(locale.clone(), catalog)
            .with_missing_policy(MissingTranslationPolicy::Placeholder);
        let number_options = NumberFormatOptions {
            minimum_fraction_digits: 2,
            maximum_fraction_digits: 2,
            grouping: true,
        };
        let datetime_options =
            DateTimeFormatOptions::new(DateFormatStyle::Long, TimeFormatStyle::Short)
                .with_offset(true);

        Self {
            greeting: translator.translate(&locale, "greeting"),
            item_count: translator.plural(&locale, "cart.items", 2),
            formatted_number: format_number(&locale, 1_234_567.5, number_options)
                .expect("the example number is finite"),
            formatted_datetime: format_datetime(
                &locale,
                1_704_196_800,
                UtcOffset::minutes(60).expect("the example offset is valid"),
                datetime_options,
            )
            .expect("the example timestamp is representable"),
            direction: locale
                .direction()
                .to_string(),
            validation_message: translator.validation_message(&locale, "required"),
            accessibility_label: translator.accessibility_label(&locale, "close"),
            locale,
        }
    }
}

impl Default for I18nExample {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for I18nExample {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        Container::new()
            .child(
                Column::new().children([
                    Text::new(format!("{} ({})", self.greeting, self.locale))
                        .wrapped()
                        .boxed(),
                    Text::new(self.item_count)
                        .wrapped()
                        .boxed(),
                    Text::new(format!("Number: {}", self.formatted_number))
                        .wrapped()
                        .boxed(),
                    Text::new(format!("Date/time: {}", self.formatted_datetime))
                        .wrapped()
                        .boxed(),
                    Text::new(format!("Direction: {}", self.direction))
                        .wrapped()
                        .boxed(),
                    Text::new(format!("Validation: {}", self.validation_message))
                        .wrapped()
                        .boxed(),
                    Text::new(format!("Accessibility: {}", self.accessibility_label))
                        .wrapped()
                        .boxed(),
                ]),
            )
            .to_element(ctx)
    }

    fn debug_name(&self) -> &'static str {
        "I18nExample"
    }
}

impl aimer::PortableWidget for I18nExample {}

/// Builds the localization example without starting an application.
pub fn i18n_example() -> impl Widget {
    I18nExample::new()
}
