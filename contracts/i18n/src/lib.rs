#![deny(missing_docs)]

//! Platform-neutral localization contracts with deterministic fallback and
//! formatting behavior.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// The inline direction used by text, layout, and logical navigation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    /// Left-to-right direction, used by most locales.
    Ltr,
    /// Right-to-left direction, used by Arabic, Hebrew, and related locales.
    Rtl,
}

impl Direction {
    /// Returns whether this direction is right-to-left.
    pub const fn is_rtl(self) -> bool {
        matches!(self, Self::Rtl)
    }
}

impl fmt::Display for Direction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Ltr => "ltr",
            Self::Rtl => "rtl",
        })
    }
}

/// A normalized language/script/region identity.
///
/// The parser intentionally covers the language, script, and region subtags
/// needed by the core fallback contract.  It rejects unmodeled extension and
/// variant subtags instead of silently changing their meaning.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct Locale {
    language: String,
    script: Option<String>,
    region: Option<String>,
}

/// Errors returned while parsing or constructing a [`Locale`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocaleError {
    /// The locale tag had no non-whitespace content.
    Empty,
    /// The language subtag was not two to eight ASCII letters.
    InvalidLanguage,
    /// The script subtag was not four ASCII letters.
    InvalidScript,
    /// The region subtag was not two ASCII letters or three ASCII digits.
    InvalidRegion,
    /// The tag contained a subtag that this focused core does not model.
    InvalidTag,
}

impl fmt::Display for LocaleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "locale tag is empty",
            Self::InvalidLanguage => "locale language must contain 2-8 ASCII letters",
            Self::InvalidScript => "locale script must contain four ASCII letters",
            Self::InvalidRegion => "locale region must contain two letters or three digits",
            Self::InvalidTag => "locale contains an unsupported or malformed subtag",
        })
    }
}

impl std::error::Error for LocaleError {}

impl Locale {
    /// Parses a BCP-47-shaped language, script, and region tag.
    ///
    /// Both `-` and `_` separators are accepted, while the stored form always
    /// uses `language-Script-REGION` casing.
    pub fn parse(tag: impl AsRef<str>) -> Result<Self, LocaleError> {
        let tag = tag.as_ref().trim();
        if tag.is_empty() {
            return Err(LocaleError::Empty);
        }

        let normalized = tag.replace('_', "-");
        let mut parts = normalized.split('-');
        let language = parts.next().ok_or(LocaleError::Empty)?;
        if !(2..=8).contains(&language.len())
            || !language.bytes().all(|byte| byte.is_ascii_alphabetic())
        {
            return Err(LocaleError::InvalidLanguage);
        }

        let mut script = None;
        let mut region = None;
        for part in parts {
            if part.is_empty() {
                return Err(LocaleError::InvalidTag);
            }
            if script.is_none()
                && part.len() == 4
                && part.bytes().all(|byte| byte.is_ascii_alphabetic())
            {
                script = Some(title_case(part));
            } else if region.is_none()
                && ((part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_alphabetic()))
                    || (part.len() == 3 && part.bytes().all(|byte| byte.is_ascii_digit())))
            {
                region = Some(part.to_ascii_uppercase());
            } else {
                return if script.is_none() && part.len() == 4 {
                    Err(LocaleError::InvalidScript)
                } else if region.is_none() && (part.len() == 2 || part.len() == 3) {
                    Err(LocaleError::InvalidRegion)
                } else {
                    Err(LocaleError::InvalidTag)
                };
            }
        }

        Ok(Self {
            language: language.to_ascii_lowercase(),
            script,
            region,
        })
    }

    /// Alias for [`Locale::parse`] that reads naturally at call sites.
    pub fn new(tag: impl AsRef<str>) -> Result<Self, LocaleError> {
        Self::parse(tag)
    }

    /// Constructs a locale from already separated language, script, and
    /// region subtags.
    pub fn from_parts(
        language: &str,
        script: Option<&str>,
        region: Option<&str>,
    ) -> Result<Self, LocaleError> {
        let mut tag = language.to_owned();
        if let Some(script) = script {
            tag.push('-');
            tag.push_str(script);
        }
        if let Some(region) = region {
            tag.push('-');
            tag.push_str(region);
        }
        Self::parse(tag)
    }

    /// Returns the normalized lowercase language subtag.
    pub fn language(&self) -> &str {
        &self.language
    }

    /// Returns the normalized title-case script subtag, if present.
    pub fn script(&self) -> Option<&str> {
        self.script.as_deref()
    }

    /// Returns the normalized uppercase region subtag, if present.
    pub fn region(&self) -> Option<&str> {
        self.region.as_deref()
    }

    /// Returns the locale's deterministic text direction.
    pub fn direction(&self) -> Direction {
        direction_for_language(&self.language)
    }

    /// Returns the ordered lookup chain from most specific to least specific.
    ///
    /// For example, `zh-Hant-TW` falls back to `zh-Hant` and then `zh`.
    pub fn fallback_chain(&self) -> Vec<Self> {
        let mut chain = Vec::with_capacity(3);
        chain.push(self.clone());
        if self.script.is_some() && self.region.is_some() {
            chain.push(Self {
                language: self.language.clone(),
                script: self.script.clone(),
                region: None,
            });
        }
        if self.region.is_some() && self.script.is_none() {
            // The region-specific locale has no additional script variant to
            // try; its language-only parent is added below.
        }
        if self.script.is_some() || self.region.is_some() {
            chain.push(Self {
                language: self.language.clone(),
                script: None,
                region: None,
            });
        }
        chain
    }

    /// Calculates the locale's integer plural category.
    pub fn plural_category(&self, value: i64) -> PluralCategory {
        plural_category(self, value)
    }
}

impl Default for Locale {
    fn default() -> Self {
        Self::parse("en-US").expect("the built-in default locale is valid")
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.language)?;
        if let Some(script) = &self.script {
            write!(formatter, "-{script}")?;
        }
        if let Some(region) = &self.region {
            write!(formatter, "-{region}")?;
        }
        Ok(())
    }
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    let mut output = String::with_capacity(value.len());
    if let Some(first) = chars.next() {
        output.push(first.to_ascii_uppercase());
    }
    output.extend(chars.map(|character| character.to_ascii_lowercase()));
    output
}

fn direction_for_language(language: &str) -> Direction {
    match language {
        "ar" | "fa" | "he" | "ur" | "ps" | "sd" | "ug" | "yi" => Direction::Rtl,
        _ => Direction::Ltr,
    }
}

/// A logical horizontal navigation intent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationIntent {
    /// Move toward the logical start or previous item.
    Previous,
    /// Move toward the logical end or next item.
    Next,
    /// Move to the logical start edge.
    Start,
    /// Move to the logical end edge.
    End,
}

/// A physical horizontal navigation direction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhysicalNavigation {
    /// Move toward the physical left edge.
    Left,
    /// Move toward the physical right edge.
    Right,
}

/// Direction-aware policy for layout-facing and keyboard-facing navigation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectionPolicy {
    direction: Direction,
    mirror_horizontal_navigation: bool,
    mirror_horizontal_gestures: bool,
}

impl DirectionPolicy {
    /// Creates the policy implied by a locale.
    pub fn for_locale(locale: &Locale) -> Self {
        Self {
            direction: locale.direction(),
            mirror_horizontal_navigation: locale.direction().is_rtl(),
            mirror_horizontal_gestures: locale.direction().is_rtl(),
        }
    }

    /// Creates an explicit left-to-right policy.
    pub const fn ltr() -> Self {
        Self {
            direction: Direction::Ltr,
            mirror_horizontal_navigation: false,
            mirror_horizontal_gestures: false,
        }
    }

    /// Creates an explicit right-to-left policy.
    pub const fn rtl() -> Self {
        Self {
            direction: Direction::Rtl,
            mirror_horizontal_navigation: true,
            mirror_horizontal_gestures: true,
        }
    }

    /// Returns the text/layout direction.
    pub const fn direction(self) -> Direction {
        self.direction
    }

    /// Returns whether this policy is right-to-left.
    pub const fn is_rtl(self) -> bool {
        self.direction.is_rtl()
    }

    /// Returns whether physical horizontal navigation is mirrored.
    pub const fn mirrors_horizontal_navigation(self) -> bool {
        self.mirror_horizontal_navigation
    }

    /// Returns whether horizontal swipe gestures should be mirrored.
    pub const fn mirrors_horizontal_gestures(self) -> bool {
        self.mirror_horizontal_gestures
    }

    /// Resolves a logical intent to a physical left/right movement.
    pub const fn physical_for(self, intent: NavigationIntent) -> PhysicalNavigation {
        let ltr = !self.mirror_horizontal_navigation;
        match (ltr, intent) {
            (true, NavigationIntent::Previous | NavigationIntent::Start)
            | (false, NavigationIntent::Next | NavigationIntent::End) => PhysicalNavigation::Left,
            _ => PhysicalNavigation::Right,
        }
    }

    /// Resolves a physical left/right movement to a logical intent.
    pub const fn logical_for(self, direction: PhysicalNavigation) -> NavigationIntent {
        match (self.mirror_horizontal_navigation, direction) {
            (false, PhysicalNavigation::Left) | (true, PhysicalNavigation::Right) => {
                NavigationIntent::Previous
            }
            _ => NavigationIntent::Next,
        }
    }
}

/// The CLDR-shaped integer plural categories used by the focused formatter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluralCategory {
    /// A locale-specific zero form.
    Zero,
    /// A locale-specific one form.
    One,
    /// A locale-specific two form.
    Two,
    /// A locale-specific few form.
    Few,
    /// A locale-specific many form.
    Many,
    /// The fallback plural form.
    Other,
}

impl PluralCategory {
    /// Returns the stable translation-key suffix for this category.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Zero => "zero",
            Self::One => "one",
            Self::Two => "two",
            Self::Few => "few",
            Self::Many => "many",
            Self::Other => "other",
        }
    }
}

/// Calculates a deterministic plural category for an integer count.
pub fn plural_category(locale: &Locale, value: i64) -> PluralCategory {
    let absolute = value.unsigned_abs();
    let modulo_10 = absolute % 10;
    let modulo_100 = absolute % 100;

    match locale.language() {
        "ar" => match absolute {
            0 => PluralCategory::Zero,
            1 => PluralCategory::One,
            2 => PluralCategory::Two,
            3..=10 => PluralCategory::Few,
            11..=99 => PluralCategory::Many,
            _ => {
                if (3..=10).contains(&(absolute % 100)) {
                    PluralCategory::Few
                } else if (11..=99).contains(&(absolute % 100)) {
                    PluralCategory::Many
                } else {
                    PluralCategory::Other
                }
            }
        },
        "ru" | "uk" | "be" => {
            if modulo_10 == 1 && modulo_100 != 11 {
                PluralCategory::One
            } else if (2..=4).contains(&modulo_10) && !(12..=14).contains(&modulo_100) {
                PluralCategory::Few
            } else if modulo_10 == 0
                || (5..=9).contains(&modulo_10)
                || (11..=14).contains(&modulo_100)
            {
                PluralCategory::Many
            } else {
                PluralCategory::Other
            }
        }
        "pl" => {
            if absolute == 1 {
                PluralCategory::One
            } else if (2..=4).contains(&modulo_10) && !(12..=14).contains(&modulo_100) {
                PluralCategory::Few
            } else {
                PluralCategory::Many
            }
        }
        "cs" | "sk" => match absolute {
            1 => PluralCategory::One,
            2..=4 => PluralCategory::Few,
            _ => PluralCategory::Other,
        },
        "sl" => match modulo_100 {
            1 => PluralCategory::One,
            2 => PluralCategory::Two,
            3 | 4 => PluralCategory::Few,
            _ => PluralCategory::Other,
        },
        "fr" | "pt" => {
            if absolute == 0 || absolute == 1 {
                PluralCategory::One
            } else {
                PluralCategory::Other
            }
        }
        "ro" => {
            if absolute == 1 {
                PluralCategory::One
            } else if absolute == 0 || (1..=19).contains(&(absolute % 100)) {
                PluralCategory::Few
            } else {
                PluralCategory::Other
            }
        }
        "lt" => {
            if modulo_10 == 1 && !(11..=19).contains(&modulo_100) {
                PluralCategory::One
            } else if (2..=9).contains(&modulo_10) && !(11..=19).contains(&modulo_100) {
                PluralCategory::Few
            } else {
                PluralCategory::Other
            }
        }
        "is" => {
            if modulo_10 == 1 && modulo_100 != 11 {
                PluralCategory::One
            } else {
                PluralCategory::Other
            }
        }
        "ja" | "ko" | "th" | "vi" | "zh" | "id" | "tr" => PluralCategory::Other,
        _ => {
            if absolute == 1 {
                PluralCategory::One
            } else {
                PluralCategory::Other
            }
        }
    }
}

/// A select category such as `male`, `female`, or `other`.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct SelectCategory(String);

impl SelectCategory {
    /// Creates a select category from an application-defined value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the category value used in the translation key.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for SelectCategory {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for SelectCategory {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for SelectCategory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// An in-memory translation catalog keyed by normalized [`Locale`] identity.
#[derive(Clone, Debug, Default)]
pub struct TranslationCatalog {
    locales: BTreeMap<Locale, BTreeMap<String, String>>,
}

impl TranslationCatalog {
    /// Creates an empty catalog.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces one translation entry.
    pub fn add_translation(
        &mut self,
        locale: Locale,
        key: impl Into<String>,
        value: impl Into<String>,
    ) {
        self.locales
            .entry(locale)
            .or_default()
            .insert(key.into(), value.into());
    }

    /// Adds a group of translation entries for one locale.
    pub fn add_translations<I, K, V>(&mut self, locale: Locale, translations: I)
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let entries = self.locales.entry(locale).or_default();
        for (key, value) in translations {
            entries.insert(key.into(), value.into());
        }
    }

    /// Returns the immutable entries for a locale, if any were registered.
    pub fn translations(&self, locale: &Locale) -> Option<&BTreeMap<String, String>> {
        self.locales.get(locale)
    }

    /// Returns the number of locales in the catalog.
    pub fn locale_count(&self) -> usize {
        self.locales.len()
    }
}

/// The result of a translation lookup, including the locale that supplied a
/// successful fallback value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TranslationLookup<'a> {
    /// A translation was found in `locale`.
    Found {
        /// The translated value.
        value: &'a str,
        /// The catalog locale that supplied `value`.
        locale: &'a Locale,
    },
    /// No translation was found through the complete fallback chain.
    Missing {
        /// The key that was requested.
        key: String,
    },
}

/// The deterministic rendering policy for a missing translation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MissingTranslationPolicy {
    /// Return the requested key as a visible fallback.
    Key,
    /// Return an empty string.
    Empty,
    /// Return `[missing:key]` to make missing content obvious in a preview.
    Placeholder,
}

impl Default for MissingTranslationPolicy {
    fn default() -> Self {
        Self::Key
    }
}

/// Locale-aware translation, plural, select, and message lookup service.
#[derive(Clone, Debug)]
pub struct Translator {
    catalog: TranslationCatalog,
    default_locale: Locale,
    missing_policy: MissingTranslationPolicy,
}

impl Translator {
    /// Creates an empty translator with an explicit default fallback locale.
    pub fn new(default_locale: Locale) -> Self {
        Self {
            catalog: TranslationCatalog::new(),
            default_locale,
            missing_policy: MissingTranslationPolicy::default(),
        }
    }

    /// Creates a translator from a catalog and explicit default locale.
    pub fn from_catalog(default_locale: Locale, catalog: TranslationCatalog) -> Self {
        Self {
            catalog,
            default_locale,
            missing_policy: MissingTranslationPolicy::default(),
        }
    }

    /// Adds or replaces one translation entry.
    pub fn add_translation(
        &mut self,
        locale: Locale,
        key: impl Into<String>,
        value: impl Into<String>,
    ) {
        self.catalog.add_translation(locale, key, value);
    }

    /// Adds a group of translation entries for one locale.
    pub fn add_translations<I, K, V>(&mut self, locale: Locale, translations: I)
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        self.catalog.add_translations(locale, translations);
    }

    /// Returns a copy of the default fallback locale.
    pub fn default_locale(&self) -> Locale {
        self.default_locale.clone()
    }

    /// Returns the current missing-translation policy.
    pub const fn missing_policy(&self) -> MissingTranslationPolicy {
        self.missing_policy
    }

    /// Sets the missing-translation policy in place.
    pub fn set_missing_policy(&mut self, policy: MissingTranslationPolicy) {
        self.missing_policy = policy;
    }

    /// Returns this translator with a different missing-translation policy.
    pub fn with_missing_policy(mut self, policy: MissingTranslationPolicy) -> Self {
        self.set_missing_policy(policy);
        self
    }

    /// Looks up a raw translation key through requested and default fallbacks.
    pub fn lookup<'a>(&'a self, locale: &Locale, key: &str) -> TranslationLookup<'a> {
        if let Some((found_locale, value)) = self.find_key(locale, key) {
            TranslationLookup::Found {
                value,
                locale: found_locale,
            }
        } else {
            TranslationLookup::Missing {
                key: key.to_owned(),
            }
        }
    }

    /// Looks up a raw key and renders it according to the missing-key policy.
    pub fn translate(&self, locale: &Locale, key: &str) -> String {
        match self.lookup(locale, key) {
            TranslationLookup::Found { value, .. } => value.to_owned(),
            TranslationLookup::Missing { key } => self.render_missing(&key),
        }
    }

    /// Builds a namespaced key without imposing a translation file format.
    pub fn message_domain_key(&self, domain: MessageDomain, key: &str) -> String {
        domain.key(key)
    }

    /// Looks up a namespaced message in the validation domain.
    pub fn validation_message(&self, locale: &Locale, key: &str) -> String {
        self.message(locale, MessageDomain::Validation, key)
    }

    /// Looks up a namespaced message in the accessibility domain.
    pub fn accessibility_label(&self, locale: &Locale, key: &str) -> String {
        self.message(locale, MessageDomain::Accessibility, key)
    }

    /// Looks up a message in an explicit domain.
    pub fn message(&self, locale: &Locale, domain: MessageDomain, key: &str) -> String {
        let full_key = domain.key(key);
        self.translate(locale, &full_key)
    }

    /// Looks up a plural translation and substitutes `{count}` in the result.
    pub fn plural(&self, locale: &Locale, key: &str, count: i64) -> String {
        let category = plural_category(locale, count);
        let mut suffixes = vec![category.as_str()];
        if category != PluralCategory::Other {
            suffixes.push(PluralCategory::Other.as_str());
        }

        for candidate_locale in self.locale_candidates(locale) {
            for suffix in &suffixes {
                let candidate_key = format!("{key}.{suffix}");
                if let Some(value) = self.find_exact(&candidate_locale, &candidate_key) {
                    return render_count(value, count);
                }
            }
            if let Some(value) = self.find_exact(&candidate_locale, key) {
                return render_count(value, count);
            }
        }
        self.render_missing(key)
    }

    /// Looks up a select translation and falls back to the `other` category.
    pub fn select(
        &self,
        locale: &Locale,
        key: &str,
        category: impl Into<SelectCategory>,
    ) -> String {
        let category = category.into();
        let selector = if category.as_str().is_empty() {
            "other"
        } else {
            category.as_str()
        };

        for candidate_locale in self.locale_candidates(locale) {
            for candidate_key in [
                format!("{key}.{selector}"),
                format!("{key}.other"),
                key.to_owned(),
            ] {
                if let Some(value) = self.find_exact(&candidate_locale, &candidate_key) {
                    return value.to_owned();
                }
            }
        }
        self.render_missing(key)
    }

    fn render_missing(&self, key: &str) -> String {
        match self.missing_policy {
            MissingTranslationPolicy::Key => key.to_owned(),
            MissingTranslationPolicy::Empty => String::new(),
            MissingTranslationPolicy::Placeholder => format!("[missing:{key}]"),
        }
    }

    fn find_key<'a>(&'a self, locale: &Locale, key: &str) -> Option<(&'a Locale, &'a str)> {
        for candidate in self.locale_candidates(locale) {
            if let Some(found) = self.find_exact(&candidate, key) {
                let found_locale = self
                    .catalog
                    .locales
                    .get_key_value(&candidate)
                    .map(|(locale, _)| locale)
                    .expect("find_exact only succeeds for a catalog locale");
                return Some((found_locale, found));
            }
        }
        None
    }

    fn find_exact<'a>(&'a self, locale: &Locale, key: &str) -> Option<&'a str> {
        self.catalog
            .locales
            .get(locale)
            .and_then(|entries| entries.get(key))
            .map(String::as_str)
    }

    fn locale_candidates(&self, requested: &Locale) -> Vec<Locale> {
        let mut candidates = Vec::with_capacity(6);
        let mut seen = BTreeSet::new();
        for candidate in requested
            .fallback_chain()
            .into_iter()
            .chain(self.default_locale.fallback_chain())
        {
            if seen.insert(candidate.clone()) {
                candidates.push(candidate);
            }
        }
        candidates
    }
}

/// Namespaces supported by the message helper methods.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageDomain {
    /// Validation errors and field-level hints.
    Validation,
    /// Accessible labels, hints, and action descriptions.
    Accessibility,
}

impl MessageDomain {
    /// Returns the stable key prefix for this domain.
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::Accessibility => "accessibility",
        }
    }

    /// Creates a namespaced key from a domain and leaf key.
    pub fn key(self, key: &str) -> String {
        format!("{}.{}", self.prefix(), key)
    }
}

fn render_count(value: &str, count: i64) -> String {
    value
        .replace("{{count}}", &count.to_string())
        .replace("{count}", &count.to_string())
}

/// Errors returned by deterministic number and date/time formatters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormatError {
    /// The number was NaN or infinite.
    NonFiniteNumber,
    /// Fractional precision exceeded the supported deterministic limit.
    FractionDigitsOutOfRange,
    /// Minimum fraction digits exceeded maximum fraction digits.
    MinimumExceedsMaximum,
    /// The date fields do not form a valid Gregorian date.
    InvalidDate,
    /// The time fields do not form a valid 24-hour time.
    InvalidTime,
    /// The fixed UTC offset is outside the range -23:59 through +23:59.
    InvalidOffset,
    /// A timestamp plus offset could not be represented safely.
    TimestampOverflow,
}

impl fmt::Display for FormatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFiniteNumber => "number must be finite",
            Self::FractionDigitsOutOfRange => "fraction digits must be at most 18",
            Self::MinimumExceedsMaximum => "minimum fraction digits exceed maximum",
            Self::InvalidDate => "invalid Gregorian date",
            Self::InvalidTime => "invalid 24-hour time",
            Self::InvalidOffset => "UTC offset must be between -23:59 and +23:59",
            Self::TimestampOverflow => "timestamp cannot be represented after applying offset",
        })
    }
}

impl std::error::Error for FormatError {}

/// Alias emphasizing that date/time operations use the same checked error
/// boundary as number formatting.
pub type DateTimeError = FormatError;

/// Options for deterministic decimal number formatting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NumberFormatOptions {
    /// Digits retained after the decimal separator even when they are zero.
    pub minimum_fraction_digits: u8,
    /// Maximum digits retained after the decimal separator.
    pub maximum_fraction_digits: u8,
    /// Whether groups of three integer digits are separated.
    pub grouping: bool,
}

impl NumberFormatOptions {
    /// Creates options with explicit minimum and maximum fraction digits.
    pub const fn new(minimum_fraction_digits: u8, maximum_fraction_digits: u8) -> Self {
        Self {
            minimum_fraction_digits,
            maximum_fraction_digits,
            grouping: true,
        }
    }

    /// Returns integer formatting options with grouping enabled.
    pub const fn integer() -> Self {
        Self {
            minimum_fraction_digits: 0,
            maximum_fraction_digits: 0,
            grouping: true,
        }
    }

    /// Enables or disables integer grouping.
    pub const fn with_grouping(mut self, grouping: bool) -> Self {
        self.grouping = grouping;
        self
    }
}

impl Default for NumberFormatOptions {
    fn default() -> Self {
        Self::new(0, 3)
    }
}

/// Formats a finite number with locale-specific separators and explicit
/// rounding options.
pub fn format_number(
    locale: &Locale,
    value: f64,
    options: NumberFormatOptions,
) -> Result<String, FormatError> {
    if !value.is_finite() {
        return Err(FormatError::NonFiniteNumber);
    }
    if options.minimum_fraction_digits > options.maximum_fraction_digits {
        return Err(FormatError::MinimumExceedsMaximum);
    }
    if options.maximum_fraction_digits > 18 {
        return Err(FormatError::FractionDigitsOutOfRange);
    }

    let negative = value.is_sign_negative() && value != 0.0;
    let raw = format!(
        "{:.*}",
        usize::from(options.maximum_fraction_digits),
        value.abs()
    );
    let (integer, fraction) = raw.split_once('.').unwrap_or((&raw, ""));
    let mut fraction = fraction.to_owned();
    while fraction.len() > usize::from(options.minimum_fraction_digits) && fraction.ends_with('0') {
        fraction.pop();
    }

    let (decimal_separator, grouping_separator) = number_symbols(locale);
    let integer = if options.grouping {
        grouped_integer(integer, grouping_separator)
    } else {
        integer.to_owned()
    };
    let mut output = String::with_capacity(integer.len() + fraction.len() + 3);
    if negative {
        output.push('-');
    }
    output.push_str(&integer);
    if !fraction.is_empty() {
        output.push(decimal_separator);
        output.push_str(&fraction);
    }
    Ok(output)
}

fn number_symbols(locale: &Locale) -> (char, char) {
    match locale.language() {
        "ar" => ('٫', '٬'),
        "de" | "es" | "it" | "nl" | "pt" | "da" | "tr" => (',', '.'),
        "fr" => (',', '\u{202f}'),
        "cs" | "fi" | "pl" | "ru" | "sv" | "uk" => (',', '\u{00a0}'),
        _ => ('.', ','),
    }
}

fn grouped_integer(integer: &str, separator: char) -> String {
    let mut output = String::with_capacity(integer.len() + integer.len() / 3);
    for (index, character) in integer.chars().enumerate() {
        if index > 0 && (integer.len() - index) % 3 == 0 {
            output.push(separator);
        }
        output.push(character);
    }
    output
}

/// A checked Gregorian calendar date without a timezone.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct CivilDate {
    year: i32,
    month: u8,
    day: u8,
}

impl CivilDate {
    /// Creates a valid Gregorian date.
    pub fn new(year: i32, month: u8, day: u8) -> Result<Self, FormatError> {
        if month == 0 || month > 12 || day == 0 || day > days_in_month(year, month) {
            return Err(FormatError::InvalidDate);
        }
        Ok(Self { year, month, day })
    }

    /// Returns the signed Gregorian year.
    pub const fn year(self) -> i32 {
        self.year
    }

    /// Returns the one-based month.
    pub const fn month(self) -> u8 {
        self.month
    }

    /// Returns the one-based day of month.
    pub const fn day(self) -> u8 {
        self.day
    }

    /// Returns whether a year is a Gregorian leap year.
    pub const fn is_leap_year(year: i32) -> bool {
        year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
    }
}

/// A checked 24-hour wall-clock time without a timezone.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct CivilTime {
    hour: u8,
    minute: u8,
    second: u8,
}

impl CivilTime {
    /// Creates a valid 24-hour time.
    pub fn new(hour: u8, minute: u8, second: u8) -> Result<Self, FormatError> {
        if hour > 23 || minute > 59 || second > 59 {
            return Err(FormatError::InvalidTime);
        }
        Ok(Self {
            hour,
            minute,
            second,
        })
    }

    /// Returns the hour in the range 0 through 23.
    pub const fn hour(self) -> u8 {
        self.hour
    }

    /// Returns the minute in the range 0 through 59.
    pub const fn minute(self) -> u8 {
        self.minute
    }

    /// Returns the second in the range 0 through 59.
    pub const fn second(self) -> u8 {
        self.second
    }
}

/// A date/time value in a named-free, fixed civil representation.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct CivilDateTime {
    date: CivilDate,
    time: CivilTime,
}

impl CivilDateTime {
    /// Creates a date/time value from checked date and time parts.
    pub const fn new(date: CivilDate, time: CivilTime) -> Self {
        Self { date, time }
    }

    /// Returns the date component.
    pub const fn date(self) -> CivilDate {
        self.date
    }

    /// Returns the time component.
    pub const fn time(self) -> CivilTime {
        self.time
    }
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if CivilDate::is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// A fixed UTC offset supplied explicitly by the caller.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct UtcOffset {
    minutes: i32,
}

impl UtcOffset {
    /// Creates an offset from a signed number of minutes.
    pub fn minutes(minutes: i32) -> Result<Self, FormatError> {
        if !(-1439..=1439).contains(&minutes) {
            return Err(FormatError::InvalidOffset);
        }
        Ok(Self { minutes })
    }

    /// Creates a whole-hour offset.
    pub fn hours(hours: i32) -> Result<Self, FormatError> {
        Self::minutes(hours.checked_mul(60).ok_or(FormatError::InvalidOffset)?)
    }

    /// Creates an offset from signed hours and minutes, applying one sign to
    /// both components.
    pub fn hours_minutes(hours: i32, minutes: i32) -> Result<Self, FormatError> {
        if minutes.unsigned_abs() >= 60 {
            return Err(FormatError::InvalidOffset);
        }
        let sign = if hours.is_negative() || minutes.is_negative() {
            -1
        } else {
            1
        };
        let magnitude = hours
            .unsigned_abs()
            .checked_mul(60)
            .and_then(|value| value.checked_add(minutes.unsigned_abs()))
            .ok_or(FormatError::InvalidOffset)?;
        let signed = i32::try_from(magnitude).map_err(|_| FormatError::InvalidOffset)? * sign;
        Self::minutes(signed)
    }

    /// Returns the signed offset in minutes.
    pub const fn as_minutes(self) -> i32 {
        self.minutes
    }

    fn iso_string(self) -> String {
        let sign = if self.minutes < 0 { '-' } else { '+' };
        let magnitude = self.minutes.unsigned_abs();
        format!("{sign}{:02}:{:02}", magnitude / 60, magnitude % 60)
    }
}

/// A date presentation style with no dependency on a system locale.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DateFormatStyle {
    /// Stable ISO-like `YYYY-MM-DD` output.
    Iso,
    /// Locale-ordered numeric output without forced zero padding.
    Numeric,
    /// Locale-ordered numeric output with two-digit month/day fields.
    Short,
    /// Locale-specific month-name output.
    Long,
}

/// A time presentation style with no dependency on a system locale.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeFormatStyle {
    /// Stable 24-hour `HH:MM:SS` output.
    Iso,
    /// Hours and minutes only.
    Short,
    /// Hours, minutes, and seconds.
    Long,
}

/// Options for combining a date, a time, and an explicit fixed offset.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DateTimeFormatOptions {
    /// Date presentation style.
    pub date_style: DateFormatStyle,
    /// Time presentation style.
    pub time_style: TimeFormatStyle,
    /// Whether to append the supplied offset as `+HH:MM` or `-HH:MM`.
    pub include_offset: bool,
}

impl DateTimeFormatOptions {
    /// Creates explicit date/time formatting options.
    pub const fn new(date_style: DateFormatStyle, time_style: TimeFormatStyle) -> Self {
        Self {
            date_style,
            time_style,
            include_offset: false,
        }
    }

    /// Returns stable ISO date/time options with an explicit offset.
    pub const fn iso() -> Self {
        Self {
            date_style: DateFormatStyle::Iso,
            time_style: TimeFormatStyle::Iso,
            include_offset: true,
        }
    }

    /// Enables or disables offset output.
    pub const fn with_offset(mut self, include_offset: bool) -> Self {
        self.include_offset = include_offset;
        self
    }
}

impl Default for DateTimeFormatOptions {
    fn default() -> Self {
        Self::new(DateFormatStyle::Long, TimeFormatStyle::Short)
    }
}

/// Formats a checked civil date using explicit locale rules.
pub fn format_date(locale: &Locale, date: CivilDate, style: DateFormatStyle) -> String {
    let year = date.year();
    let month = date.month();
    let day = date.day();
    match style {
        DateFormatStyle::Iso => format!("{year:04}-{month:02}-{day:02}"),
        DateFormatStyle::Numeric => {
            if month_first(locale) {
                format!("{month}/{day}/{year}")
            } else {
                format!("{day}/{month}/{year}")
            }
        }
        DateFormatStyle::Short => {
            if month_first(locale) {
                format!("{month:02}/{day:02}/{year:04}")
            } else {
                format!("{day:02}/{month:02}/{year:04}")
            }
        }
        DateFormatStyle::Long => long_date(locale, date),
    }
}

fn month_first(locale: &Locale) -> bool {
    match locale.language() {
        "ja" | "zh" => true,
        "en" => !matches!(locale.region(), Some("GB" | "IE" | "AU" | "NZ")),
        _ => false,
    }
}

fn long_date(locale: &Locale, date: CivilDate) -> String {
    let month_name = month_name(locale.language(), date.month());
    match locale.language() {
        "en" if locale.region() == Some("US") || locale.region().is_none() => {
            format!("{month_name} {}, {}", date.day(), date.year())
        }
        "ja" | "zh" => format!("{}年{}月{}日", date.year(), date.month(), date.day()),
        "de" => format!("{}. {} {}", date.day(), month_name, date.year()),
        "es" => format!("{} de {} de {}", date.day(), month_name, date.year()),
        _ => format!("{} {} {}", date.day(), month_name, date.year()),
    }
}

fn month_name(language: &str, month: u8) -> &'static str {
    const ENGLISH: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    const FRENCH: [&str; 12] = [
        "janvier",
        "février",
        "mars",
        "avril",
        "mai",
        "juin",
        "juillet",
        "août",
        "septembre",
        "octobre",
        "novembre",
        "décembre",
    ];
    const GERMAN: [&str; 12] = [
        "Januar",
        "Februar",
        "März",
        "April",
        "Mai",
        "Juni",
        "Juli",
        "August",
        "September",
        "Oktober",
        "November",
        "Dezember",
    ];
    const SPANISH: [&str; 12] = [
        "enero",
        "febrero",
        "marzo",
        "abril",
        "mayo",
        "junio",
        "julio",
        "agosto",
        "septiembre",
        "octubre",
        "noviembre",
        "diciembre",
    ];
    let names = match language {
        "fr" => &FRENCH,
        "de" => &GERMAN,
        "es" => &SPANISH,
        _ => &ENGLISH,
    };
    names[usize::from(month - 1)]
}

/// Formats a checked civil time using explicit locale rules.
pub fn format_time(locale: &Locale, time: CivilTime, style: TimeFormatStyle) -> String {
    let use_12_hour = matches!(style, TimeFormatStyle::Short | TimeFormatStyle::Long)
        && locale.language() == "en"
        && !matches!(locale.region(), Some("GB" | "IE" | "AU" | "NZ"));
    if use_12_hour {
        let hour = match time.hour() % 12 {
            0 => 12,
            hour => hour,
        };
        let meridiem = if time.hour() < 12 { "AM" } else { "PM" };
        if matches!(style, TimeFormatStyle::Short) {
            format!("{hour}:{:02} {meridiem}", time.minute())
        } else {
            format!(
                "{hour}:{:02}:{:02} {meridiem}",
                time.minute(),
                time.second()
            )
        }
    } else if matches!(style, TimeFormatStyle::Short) {
        format!("{:02}:{:02}", time.hour(), time.minute())
    } else {
        format!(
            "{:02}:{:02}:{:02}",
            time.hour(),
            time.minute(),
            time.second()
        )
    }
}

/// Converts a Unix timestamp in seconds using the supplied fixed UTC offset.
pub fn datetime_from_timestamp(
    timestamp_seconds: i64,
    offset: UtcOffset,
) -> Result<CivilDateTime, FormatError> {
    let offset_seconds = i64::from(offset.as_minutes()) * 60;
    let local_seconds = timestamp_seconds
        .checked_add(offset_seconds)
        .ok_or(FormatError::TimestampOverflow)?;
    let days = local_seconds.div_euclid(86_400);
    let seconds = local_seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let year = i32::try_from(year).map_err(|_| FormatError::TimestampOverflow)?;
    let date = CivilDate::new(year, month, day)?;
    let time = CivilTime::new(
        (seconds / 3_600) as u8,
        ((seconds % 3_600) / 60) as u8,
        (seconds % 60) as u8,
    )?;
    Ok(CivilDateTime::new(date, time))
}

// Proleptic Gregorian conversion based on a 1970-01-01 day zero.  It uses
// only integer arithmetic, so output does not depend on a host timezone or
// system calendar implementation.
fn civil_from_days(days: i64) -> (i64, u8, u8) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month as u8, day as u8)
}

/// Formats a Unix timestamp after applying an explicit fixed UTC offset.
pub fn format_datetime(
    locale: &Locale,
    timestamp_seconds: i64,
    offset: UtcOffset,
    options: DateTimeFormatOptions,
) -> Result<String, FormatError> {
    let datetime = datetime_from_timestamp(timestamp_seconds, offset)?;
    let date = format_date(locale, datetime.date(), options.date_style);
    let time = format_time(locale, datetime.time(), options.time_style);
    let separator = if options.date_style == DateFormatStyle::Iso {
        "T"
    } else if locale.language() == "en" {
        ", "
    } else {
        " "
    };
    let mut output = format!("{date}{separator}{time}");
    if options.include_offset {
        if options.date_style != DateFormatStyle::Iso {
            output.push(' ');
        }
        output.push_str(&offset.iso_string());
    }
    Ok(output)
}

/// Alias for [`format_datetime`] using timestamp terminology.
pub fn format_timestamp(
    locale: &Locale,
    timestamp_seconds: i64,
    offset: UtcOffset,
    options: DateTimeFormatOptions,
) -> Result<String, FormatError> {
    format_datetime(locale, timestamp_seconds, offset, options)
}
