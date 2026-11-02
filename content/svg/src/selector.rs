use std::str::FromStr;
use std::sync::Arc;

use aimer_cupid::svg::SvgNode;

use crate::SvgError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SvgSelector {
    Id(Arc<str>),
    Class(Arc<str>),
    Element(Arc<str>),
}

impl SvgSelector {
    pub(crate) fn matches(&self, node: &SvgNode) -> bool {
        match self {
            Self::Id(id) => node.svg_id.as_deref() == Some(id),
            Self::Class(class) => node
                .classes
                .iter()
                .any(|value| value.as_ref() == class.as_ref()),
            Self::Element(element) => match node.element {
                aimer_cupid::svg::SvgElementKind::Group => element.as_ref() == "g",
                aimer_cupid::svg::SvgElementKind::Path => element.as_ref() == "path",
            },
        }
    }
}

impl FromStr for SvgSelector {
    type Err = SvgError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        if let Some(id) = value.strip_prefix('#') {
            validate_selector_name(id, value).map(|_| Self::Id(Arc::from(id)))
        } else if let Some(class) = value.strip_prefix('.') {
            validate_selector_name(class, value).map(|_| Self::Class(Arc::from(class)))
        } else {
            validate_selector_name(value, value)
                .map(|_| Self::Element(Arc::from(value.to_ascii_lowercase())))
        }
    }
}

fn validate_selector_name(name: &str, original: &str) -> Result<(), SvgError> {
    if name.is_empty() || name.chars().any(char::is_whitespace) {
        Err(SvgError::InvalidSelector(original.to_owned()))
    } else {
        Ok(())
    }
}

impl TryFrom<&str> for SvgSelector {
    type Error = SvgError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl TryFrom<String> for SvgSelector {
    type Error = SvgError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}
