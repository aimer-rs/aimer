use aimer_i18n::{
    CivilDate, DateFormatStyle, DateTimeFormatOptions, Direction, DirectionPolicy, Locale,
    MessageDomain, MissingTranslationPolicy, NavigationIntent, NumberFormatOptions,
    PhysicalNavigation, PluralCategory, SelectCategory, TimeFormatStyle, TranslationCatalog,
    TranslationLookup, Translator, UtcOffset, format_date, format_datetime, format_number,
    plural_category,
};

fn locale(tag: &str) -> Locale {
    Locale::parse(tag).expect("test locale is valid")
}

#[test]
fn locale_identity_and_fallback_chain_are_deterministic() {
    let locale = locale("zh-hant-tw");

    assert_eq!(locale.to_string(), "zh-Hant-TW");
    assert_eq!(locale.language(), "zh");
    assert_eq!(locale.script(), Some("Hant"));
    assert_eq!(locale.region(), Some("TW"));
    assert_eq!(
        locale
            .fallback_chain()
            .into_iter()
            .map(|item| item.to_string())
            .collect::<Vec<_>>(),
        vec!["zh-Hant-TW", "zh-Hant", "zh"]
    );
    assert_eq!(locale.direction(), Direction::Ltr);
}

#[test]
fn translation_lookup_uses_fallback_and_explicit_missing_policy() {
    let mut catalog = TranslationCatalog::new();
    catalog.add_translations(locale("en"), [("greeting", "Hello")]);
    catalog.add_translations(locale("fr"), [("greeting", "Bonjour")]);

    let mut translator = Translator::from_catalog(locale("en-US"), catalog);
    let requested = locale("en-GB");

    assert_eq!(translator.translate(&requested, "greeting"), "Hello");
    assert!(matches!(
        translator.lookup(&requested, "greeting"),
        TranslationLookup::Found { value: "Hello", .. }
    ));

    assert_eq!(translator.translate(&requested, "unknown"), "unknown");
    translator.set_missing_policy(MissingTranslationPolicy::Placeholder);
    assert_eq!(
        translator.translate(&requested, "unknown"),
        "[missing:unknown]"
    );
    translator.set_missing_policy(MissingTranslationPolicy::Empty);
    assert_eq!(translator.translate(&requested, "unknown"), "");
}

#[test]
fn plural_and_select_lookup_cover_categories_and_fallback() {
    let mut catalog = TranslationCatalog::new();
    catalog.add_translations(
        locale("en"),
        [
            ("cart.items.one", "{count} item"),
            ("cart.items.other", "{count} items"),
            ("invite.male", "He joined"),
            ("invite.other", "They joined"),
        ],
    );
    let translator = Translator::from_catalog(locale("en"), catalog);

    assert_eq!(
        translator.plural(&locale("en-US"), "cart.items", 0),
        "0 items"
    );
    assert_eq!(
        translator.plural(&locale("en-US"), "cart.items", 1),
        "1 item"
    );
    assert_eq!(
        translator.plural(&locale("en-US"), "cart.items", 2),
        "2 items"
    );
    assert_eq!(
        translator.select(&locale("en-US"), "invite", SelectCategory::new("female")),
        "They joined"
    );
    assert_eq!(
        translator.validation_message(&locale("en-US"), "required"),
        "validation.required"
    );
    assert_eq!(
        translator.message_domain_key(MessageDomain::Accessibility, "close"),
        "accessibility.close"
    );
}

#[test]
fn plural_rules_cover_zero_one_few_many_boundaries_without_system_locale() {
    let english = locale("en");
    let french = locale("fr");
    let russian = locale("ru");
    let arabic = locale("ar");

    assert_eq!(plural_category(&english, 0), PluralCategory::Other);
    assert_eq!(plural_category(&english, 1), PluralCategory::One);
    assert_eq!(plural_category(&french, 0), PluralCategory::One);
    assert_eq!(plural_category(&russian, 1), PluralCategory::One);
    assert_eq!(plural_category(&russian, 2), PluralCategory::Few);
    assert_eq!(plural_category(&russian, 5), PluralCategory::Many);
    assert_eq!(plural_category(&russian, 21), PluralCategory::One);
    assert_eq!(plural_category(&russian, 11), PluralCategory::Many);
    assert_eq!(plural_category(&arabic, 0), PluralCategory::Zero);
    assert_eq!(plural_category(&arabic, 1), PluralCategory::One);
    assert_eq!(plural_category(&arabic, 2), PluralCategory::Two);
    assert_eq!(plural_category(&arabic, 7), PluralCategory::Few);
    assert_eq!(plural_category(&arabic, 15), PluralCategory::Many);
}

#[test]
fn number_formatting_is_locale_explicit_and_rejects_non_finite_values() {
    let options = NumberFormatOptions {
        minimum_fraction_digits: 2,
        maximum_fraction_digits: 2,
        grouping: true,
    };

    assert_eq!(
        format_number(&locale("en-US"), 1_234_567.5, options).unwrap(),
        "1,234,567.50"
    );
    assert_eq!(
        format_number(&locale("de-DE"), 1_234_567.5, options).unwrap(),
        "1.234.567,50"
    );
    assert_eq!(
        format_number(&locale("fr-FR"), 1_234_567.5, options).unwrap(),
        "1\u{202f}234\u{202f}567,50"
    );
    assert!(format_number(&locale("en"), f64::NAN, options).is_err());
}

#[test]
fn date_and_time_formatting_uses_the_supplied_fixed_offset() {
    let date = CivilDate::new(2024, 1, 2).unwrap();
    assert_eq!(
        format_date(&locale("en-US"), date, DateFormatStyle::Iso),
        "2024-01-02"
    );

    let options =
        DateTimeFormatOptions::new(DateFormatStyle::Iso, TimeFormatStyle::Long).with_offset(true);
    assert_eq!(
        format_datetime(
            &locale("en-GB"),
            0,
            UtcOffset::minutes(150).unwrap(),
            options
        )
        .unwrap(),
        "1970-01-01T02:30:00+02:30"
    );
    assert_eq!(
        format_datetime(
            &locale("en-GB"),
            0,
            UtcOffset::minutes(-300).unwrap(),
            options
        )
        .unwrap(),
        "1969-12-31T19:00:00-05:00"
    );
}

#[test]
fn non_iso_offset_and_region_date_order_are_explicit() {
    let date = CivilDate::new(2024, 1, 2).unwrap();
    assert_eq!(
        format_date(&locale("en-GB"), date, DateFormatStyle::Short),
        "02/01/2024"
    );

    let options = DateTimeFormatOptions::new(DateFormatStyle::Short, TimeFormatStyle::Short)
        .with_offset(true);
    assert_eq!(
        format_datetime(&locale("fr-FR"), 0, UtcOffset::hours(1).unwrap(), options).unwrap(),
        "01/01/1970 01:00 +01:00"
    );
}

#[test]
fn rtl_direction_policy_mirrors_horizontal_navigation() {
    let ltr = DirectionPolicy::for_locale(&locale("en"));
    let rtl = DirectionPolicy::for_locale(&locale("ar"));

    assert!(!ltr.is_rtl());
    assert!(rtl.is_rtl());
    assert!(!ltr.mirrors_horizontal_navigation());
    assert!(rtl.mirrors_horizontal_navigation());
    assert_eq!(
        ltr.physical_for(NavigationIntent::Previous),
        PhysicalNavigation::Left
    );
    assert_eq!(
        rtl.physical_for(NavigationIntent::Previous),
        PhysicalNavigation::Right
    );
    assert_eq!(
        rtl.physical_for(NavigationIntent::Next),
        PhysicalNavigation::Left
    );
    assert_eq!(
        rtl.logical_for(PhysicalNavigation::Right),
        NavigationIntent::Previous
    );
}

#[test]
fn validation_and_accessibility_messages_are_namespaced_without_a_file_format() {
    let mut catalog = TranslationCatalog::new();
    catalog.add_translations(
        locale("es"),
        [
            ("validation.required", "Este campo es obligatorio"),
            ("accessibility.close", "Cerrar"),
        ],
    );
    let translator = Translator::from_catalog(locale("es"), catalog);

    assert_eq!(
        translator.validation_message(&locale("es-MX"), "required"),
        "Este campo es obligatorio"
    );
    assert_eq!(
        translator.accessibility_label(&locale("es-MX"), "close"),
        "Cerrar"
    );
}
