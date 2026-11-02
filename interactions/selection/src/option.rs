use std::collections::HashSet;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::ops::Deref;
use std::rc::Rc;

/// An option shared by radio groups, selects, and autocompletes.
///
/// `key` is the stable identity used by pointer and overlay adapters. Labels
/// are presentation text and may intentionally repeat. Keys and labels are
/// reference counted, so handing an option to a popup row costs two counter
/// bumps instead of two string copies.
#[derive(Clone, Debug, PartialEq)]
pub struct ChoiceOption<T> {
    key: Rc<str>,
    label: Rc<str>,
    value: T,
    disabled: bool,
}

impl<T> ChoiceOption<T> {
    /// Creates an enabled option with a stable key, display label, and value.
    #[inline]
    pub fn new(key: impl Into<Rc<str>>, label: impl Into<Rc<str>>, value: T) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            value,
            disabled: false,
        }
    }

    /// Sets whether this option can be activated.
    #[inline]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Returns the stable option key.
    #[inline]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Returns the display label. Labels do not have to be unique.
    #[inline]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the caller-owned option value.
    #[inline]
    pub fn value(&self) -> &T {
        &self.value
    }

    /// Returns whether this option cannot be activated.
    #[inline]
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    #[inline]
    pub(crate) fn shared_key(&self) -> Rc<str> {
        Rc::clone(&self.key)
    }

}

/// An invalid option collection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptionError {
    /// An option key was empty and could not provide stable identity.
    EmptyKey,
    /// Two options used the same stable key.
    DuplicateKey(String),
}

impl Display for OptionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyKey => formatter.write_str("choice option keys cannot be empty"),
            Self::DuplicateKey(key) => write!(formatter, "duplicate choice option key: {key}"),
        }
    }
}

impl Error for OptionError {}

/// A validated, cheaply shareable option list.
///
/// Controls keep their options here so a rebuild, a popup, and every row can
/// share one allocation. Validation runs once, when the list is built, in time
/// linear in the number of options.
pub(crate) struct Options<T>(Rc<[ChoiceOption<T>]>);

impl<T> Clone for Options<T> {
    #[inline]
    fn clone(&self) -> Self {
        Self(Rc::clone(&self.0))
    }
}

impl<T> Default for Options<T> {
    #[inline]
    fn default() -> Self {
        Self(Rc::from(Vec::new()))
    }
}

impl<T> Deref for Options<T> {
    type Target = [ChoiceOption<T>];

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> Options<T> {
    /// Collects `options`, rejecting empty and duplicate keys.
    pub(crate) fn try_new(
        options: impl IntoIterator<Item = ChoiceOption<T>>,
    ) -> Result<Self, OptionError> {
        let options: Rc<[ChoiceOption<T>]> = options.into_iter().collect();
        let mut seen = HashSet::with_capacity(options.len());
        for option in options.iter() {
            if option.key.is_empty() {
                return Err(OptionError::EmptyKey);
            }
            if !seen.insert(&*option.key) {
                return Err(OptionError::DuplicateKey(option.key.to_string()));
            }
        }
        Ok(Self(options))
    }

    /// Whether both lists are the same allocation, which is the common
    /// "nothing changed" answer after a rebuild.
    #[inline]
    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    #[inline]
    pub(crate) fn position_of_value(&self, value: &T) -> Option<usize>
    where
        T: PartialEq,
    {
        self.iter().position(|option| option.value() == value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_keys_are_rejected_but_duplicate_labels_are_allowed() {
        let duplicate_keys = Options::try_new([
            ChoiceOption::new("a", "One", 1),
            ChoiceOption::new("a", "Two", 2),
        ]);
        assert_eq!(
            duplicate_keys.err(),
            Some(OptionError::DuplicateKey("a".to_owned()))
        );

        let duplicate_labels = Options::try_new([
            ChoiceOption::new("a", "Same", 1),
            ChoiceOption::new("b", "Same", 2),
        ]);
        assert!(duplicate_labels.is_ok());
    }

    #[test]
    fn empty_keys_are_rejected() {
        let options = Options::try_new([ChoiceOption::new("", "Nameless", 1)]);
        assert_eq!(options.err(), Some(OptionError::EmptyKey));
    }

    #[test]
    fn a_clone_shares_the_allocation() {
        let options = Options::try_new([ChoiceOption::new("a", "One", 1)]).unwrap();
        assert!(options.ptr_eq(&options.clone()));
    }

    #[test]
    fn options_are_found_by_value() {
        let options = Options::try_new([
            ChoiceOption::new("a", "One", 1),
            ChoiceOption::new("b", "Two", 2),
        ])
        .unwrap();
        assert_eq!(options.position_of_value(&2), Some(1));
        assert_eq!(options.position_of_value(&9), None);
    }
}
