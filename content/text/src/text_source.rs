use std::fmt;
use std::ops::Deref;
use std::rc::Rc;

use aimer_rubick::ShareRef;
use aimer_widget::portable::__anteros::{PropertyId, PropertyValue};
#[cfg(feature = "portable-guest")]
use aimer_widget::portable::{PortableBuildContext, PortableBuildError, PortableEncodeProperty};
use aimer_widget::portable::{
    PortableMaterializeError, PortableMaterializeProperty, PortableProperty,
    PortablePropertyReflection,
};

/// A text payload that is either a `'static` literal or shared string data.
///
/// [`Text`](crate::Text) and [`RawTextWidget`](crate::RawTextWidget) rebuild
/// every frame, so the common case — a label built from a string literal —
/// must not pay an allocation on every rebuild. `Rc<str>` cannot help a static
/// `&str` here:
/// constructing one from a `&str` or `String` always allocates and copies,
/// because its layout places the strong/weak counters next to the string
/// bytes in one block. `TextSource` instead keeps the `&'static str` case as a
/// bare pointer, uses `Rc<str>` for directly shared strings, and retains a
/// [`ShareRef<str>`] when the text is projected from another shared owner.
///
/// # Example
///
/// ```
/// use std::rc::Rc;
///
/// use aimer_text::{ShareRef, Text};
///
/// // No allocation: the literal is stored as a `&'static str`.
/// let label = Text::new("Aimer");
///
/// // One allocation, same as constructing an `Rc<str>` directly.
/// let counter = Text::new(format!("Count: {}", 3));
///
/// // A shared string retains its source without copying its bytes.
/// let shared: Rc<str> = Rc::from("Shared label");
/// let shared_label = Text::new(ShareRef::from_rc(&shared));
/// ```
#[derive(Clone)]
pub enum TextSource {
    /// A string literal or other data that lives for the entire program.
    ///
    /// Constructing this variant is free: it stores the pointer and length
    /// directly, with no allocation and no refcount.
    Static(&'static str),
    /// Owned string data shared through a reference count.
    ///
    /// Cloning this variant is a refcount bump. Constructing it from a
    /// `String` allocates once, exactly like building an `Rc<str>` directly.
    Shared(Rc<str>),
    /// An owning, read-only projected string. Cloning this variant clones the
    /// shared handle without copying the selected string.
    ShareRef(ShareRef<str>),
}

impl TextSource {
    /// Borrows the underlying string, regardless of which variant this is.
    #[inline]
    pub fn as_str(&self) -> &str {
        match self {
            TextSource::Static(text) => text,
            TextSource::Shared(text) => text,
            TextSource::ShareRef(text) => text,
        }
    }

    /// Returns a reference-counted handle to this text.
    ///
    /// [`TextSource::Shared`] is cloned for free; [`TextSource::Static`]
    /// allocates once to produce the handle. [`TextSource::ShareRef`] copies
    /// into a new `Rc<str>` when a concrete handle is required. Use this only
    /// at that boundary — for example, when invoking a link callback — rather
    /// than on every rebuild.
    #[inline]
    pub fn to_rc(&self) -> Rc<str> {
        match self {
            TextSource::Static(text) => Rc::from(*text),
            TextSource::Shared(text) => Rc::clone(text),
            TextSource::ShareRef(text) => Rc::from(text.as_ref()),
        }
    }
}

impl Default for TextSource {
    #[inline]
    fn default() -> Self {
        TextSource::Static("")
    }
}

impl Deref for TextSource {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for TextSource {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for TextSource {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for TextSource {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl PartialEq for TextSource {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for TextSource {}

impl From<&'static str> for TextSource {
    #[inline]
    fn from(text: &'static str) -> Self {
        TextSource::Static(text)
    }
}

impl From<String> for TextSource {
    #[inline]
    fn from(text: String) -> Self {
        TextSource::Shared(text.into())
    }
}

impl From<Rc<str>> for TextSource {
    #[inline]
    fn from(text: Rc<str>) -> Self {
        TextSource::Shared(text)
    }
}

impl From<ShareRef<str>> for TextSource {
    #[inline]
    fn from(text: ShareRef<str>) -> Self {
        TextSource::ShareRef(text)
    }
}

impl From<ShareRef<String>> for TextSource {
    #[inline]
    fn from(text: ShareRef<String>) -> Self {
        TextSource::from(text.project(String::as_str))
    }
}

impl PortableProperty for TextSource {
    const REFLECTION: PortablePropertyReflection = PortablePropertyReflection::string_ref();
}

impl PortableMaterializeProperty for TextSource {
    fn from_awir(
        document: &aimer_widget::portable::__anteros::WidgetDocumentView<'_>,
        property: PropertyId,
        value: PropertyValue,
    ) -> Result<Self, PortableMaterializeError> {
        let PropertyValue::StringRef(index) = value else {
            return Err(PortableMaterializeError::InvalidPropertyType { property });
        };
        document
            .string(index)
            .map(|text| Self::from(text.to_owned()))
            .ok_or(PortableMaterializeError::InvalidPropertyReference { property, index })
    }
}

#[cfg(feature = "portable-guest")]
impl PortableEncodeProperty for TextSource {
    fn encode_property(
        self,
        context: &mut PortableBuildContext,
    ) -> Result<PropertyValue, PortableBuildError> {
        context.push_string(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aimer_rubick::{ShareRef, Shared};

    #[test]
    fn static_text_borrows_without_allocating_an_rc() {
        let source = TextSource::from("literal");
        assert!(matches!(source, TextSource::Static("literal")));
        assert_eq!(&*source, "literal");
    }

    #[test]
    fn owned_text_is_stored_as_shared() {
        let source = TextSource::from(String::from("owned"));
        assert!(matches!(source, TextSource::Shared(_)));
        assert_eq!(&*source, "owned");
    }

    #[test]
    fn to_rc_reuses_an_existing_allocation() {
        let rc: Rc<str> = Rc::from("shared");
        let source = TextSource::from(Rc::clone(&rc));
        assert!(Rc::ptr_eq(&rc, &source.to_rc()));
    }

    #[test]
    fn to_rc_allocates_once_for_static_text() {
        let source = TextSource::from("literal");
        let rc = source.to_rc();
        assert_eq!(&*rc, "literal");
    }

    #[test]
    fn shared_ref_from_rc_keeps_text_alive_after_the_source_rc_is_dropped() {
        let rc: Rc<str> = Rc::from("shared-ref");
        let source = TextSource::from(ShareRef::from_rc(&rc));

        drop(rc);

        assert_eq!(source.as_str(), "shared-ref");
    }

    #[test]
    fn projected_shared_string_keeps_its_owner_alive_without_copying_the_text() {
        struct State {
            title: String,
        }

        let state = Shared::new(State {
            title: String::from("projected"),
        });
        let title = state.project(|state| &state.title).project(String::as_str);
        let source = TextSource::from(title);

        drop(state);

        assert_eq!(source.as_str(), "projected");
    }

    #[test]
    fn shared_string_is_projected_to_text_without_copying_the_source_value() {
        let value = Rc::new(String::from("shared-string"));
        let source = TextSource::from(ShareRef::from_rc(&value));

        drop(value);

        assert_eq!(source.as_str(), "shared-string");
    }

    #[test]
    fn equality_compares_by_content_across_variants() {
        assert_eq!(
            TextSource::from("same"),
            TextSource::from(String::from("same"))
        );
    }
}
