use std::rc::Rc;

use aimer_rubick::ShareRef;
use aimer_widget::AnyWidget;
use aimer_widget::base::BuildContext;

use crate::Document;

/// Describes the delimiters surrounding a custom block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockSyntax {
    /// A block whose opening and closing delimiters occupy their own lines.
    Paired {
        opening: &'static str,
        closing: &'static str,
    },
}

impl BlockSyntax {
    pub(crate) fn delimiters(self) -> (&'static str, &'static str) {
        match self {
            Self::Paired { opening, closing } => (opening, closing),
        }
    }
}

/// Identifies and describes a custom block syntax.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockRule {
    name: ShareRef<str>,
    syntax: BlockSyntax,
}

impl BlockRule {
    /// Creates a named custom block rule.
    #[inline]
    pub fn new(name: impl Into<String>, syntax: BlockSyntax) -> Self {
        Self {
            name: ShareRef::from(name.into()),
            syntax,
        }
    }

    /// Returns the name passed to custom block builders.
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn shared_name(&self) -> ShareRef<str> {
        self.name.clone()
    }

    pub(crate) fn delimiters(&self) -> (&'static str, &'static str) {
        self.syntax.delimiters()
    }
}

impl From<BlockSyntax> for BlockRule {
    fn from(syntax: BlockSyntax) -> Self {
        let (opening, _) = syntax.delimiters();
        Self::new(opening, syntax)
    }
}

/// Describes the delimiters surrounding a custom inline value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineSyntax {
    /// An inline value surrounded by an opening and closing delimiter.
    Paired {
        opening: &'static str,
        closing: &'static str,
    },
}

impl InlineSyntax {
    pub(crate) fn delimiters(self) -> (&'static str, &'static str) {
        match self {
            Self::Paired { opening, closing } => (opening, closing),
        }
    }
}

/// Identifies and describes a custom inline syntax.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineRule {
    name: ShareRef<str>,
    syntax: InlineSyntax,
}

impl InlineRule {
    /// Creates a named custom inline rule.
    #[inline]
    pub fn new(name: impl Into<String>, syntax: InlineSyntax) -> Self {
        Self {
            name: ShareRef::from(name.into()),
            syntax,
        }
    }

    /// Returns the name passed to custom inline builders.
    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn shared_name(&self) -> ShareRef<str> {
        self.name.clone()
    }

    pub(crate) fn delimiters(&self) -> (&'static str, &'static str) {
        self.syntax.delimiters()
    }
}

impl From<InlineSyntax> for InlineRule {
    fn from(syntax: InlineSyntax) -> Self {
        let (opening, _) = syntax.delimiters();
        Self::new(opening, syntax)
    }
}

/// Parsed data passed to a custom block builder.
#[derive(Clone, Debug, PartialEq)]
pub struct CustomBlockData {
    /// The name of the rule that matched this block.
    pub name: ShareRef<str>,
    /// The source between the opening and closing delimiters.
    pub text: ShareRef<str>,
    /// The block body parsed as Markdown, including nested custom syntax.
    pub content: Document,
}

/// Parsed data passed to a custom inline builder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomInlineData {
    /// The name of the rule that matched this value.
    pub name: ShareRef<str>,
    /// The source between the opening and closing delimiters.
    pub text: ShareRef<str>,
    /// An alias for [`CustomInlineData::text`] useful for label-like syntax.
    pub label: ShareRef<str>,
}

/// A callback that turns parsed custom block data into a widget.
pub type CustomBlockBuilder = Rc<dyn Fn(&CustomBlockData) -> AnyWidget>;

/// A callback that turns parsed custom inline data into a widget.
pub type CustomInlineBuilder = Rc<dyn Fn(&CustomInlineData) -> AnyWidget>;

/// Input passed to a strongly typed custom block parser.
pub struct CustomBlockInput<'a> {
    /// The source between the opening and closing block delimiters.
    pub raw: &'a str,
    /// The block source parsed as Markdown, including nested custom syntax.
    pub content: &'a Document,
}

/// Defines a strongly typed custom Markdown block.
pub trait CustomBlock: 'static {
    /// The unique name identifying this block.
    const NAME: &'static str;
    /// The opening delimiter that identifies this block.
    const OPENING: &'static str;
    /// The closing delimiter for this block.
    const CLOSING: &'static str = ":::";

    /// The parsed representation consumed by [`Self::build`].
    type Props: 'static;

    /// Parses the raw block source and its nested Markdown content.
    fn parse(input: CustomBlockInput<'_>) -> Result<Self::Props, crate::MarkdownError>;

    /// Builds the widget for parsed block properties.
    fn build(props: &Self::Props, ctx: &BuildContext) -> AnyWidget;
}

/// Defines a strongly typed custom Markdown inline element.
pub trait CustomInline: 'static {
    /// The unique name identifying this inline element.
    const NAME: &'static str;
    /// The opening delimiter that identifies this inline element.
    const OPENING: &'static str;
    /// The closing delimiter for this inline element.
    const CLOSING: &'static str;

    /// The parsed representation consumed by [`Self::build`].
    type Props: 'static;

    /// Parses the source between the inline delimiters.
    fn parse(raw: &str) -> Result<Self::Props, crate::MarkdownError>;

    /// Builds the widget for parsed inline properties.
    fn build(props: &Self::Props, ctx: &BuildContext) -> AnyWidget;
}

/// A typed block callback erased at the renderer boundary.
pub(crate) type TypedCustomBlockBuilder = Rc<dyn Fn(&CustomBlockData, &BuildContext) -> AnyWidget>;

/// A typed inline callback erased at the renderer boundary.
pub(crate) type TypedCustomInlineBuilder =
    Rc<dyn Fn(&CustomInlineData, &BuildContext) -> AnyWidget>;

#[cfg(test)]
mod tests {
    use aimer_text::Text;
    use aimer_widget::Widget;
    use aimer_widget::base::BuildContext;

    use super::{CustomBlock, CustomBlockInput, CustomInline};
    use crate::{Document, MarkdownError};

    struct Alert;

    #[derive(Debug, PartialEq)]
    struct AlertProps {
        title: String,
        body: Document,
    }

    impl CustomBlock for Alert {
        const NAME: &'static str = "alert";
        const OPENING: &'static str = ":::alert";

        type Props = AlertProps;

        fn parse(input: CustomBlockInput<'_>) -> Result<Self::Props, MarkdownError> {
            Ok(AlertProps {
                title: input.raw.trim().to_owned(),
                body: input.content.clone(),
            })
        }

        fn build(_props: &Self::Props, _ctx: &BuildContext) -> aimer_widget::AnyWidget {
            Text::new("alert").boxed()
        }
    }

    struct Mention;

    impl CustomInline for Mention {
        const NAME: &'static str = "mention";
        const OPENING: &'static str = "@{";
        const CLOSING: &'static str = "}";

        type Props = String;

        fn parse(raw: &str) -> Result<Self::Props, MarkdownError> {
            let name = raw.trim();
            (!name.is_empty())
                .then(|| name.to_owned())
                .ok_or_else(|| MarkdownError::new("mention cannot be empty"))
        }

        fn build(_props: &Self::Props, _ctx: &BuildContext) -> aimer_widget::AnyWidget {
            Text::new("mention").boxed()
        }
    }

    #[test]
    fn typed_rules_expose_their_static_syntax_and_decode_input() {
        let document = Document::parse("nested").expect("fixture should parse");
        let props = Alert::parse(CustomBlockInput {
            raw: " Critical ",
            content: &document,
        })
        .expect("alert should parse");

        assert_eq!(Alert::NAME, "alert");
        assert_eq!(Alert::OPENING, ":::alert");
        assert_eq!(Alert::CLOSING, ":::");
        assert_eq!(props.title, "Critical");
        assert_eq!(props.body, document);
        assert_eq!(Mention::parse("alice").unwrap(), "alice");
    }

    #[test]
    fn typed_inline_rules_reject_invalid_payloads() {
        let error = Mention::parse(" ").expect_err("empty mentions must be rejected");

        assert_eq!(error.message(), "mention cannot be empty");
    }
}
