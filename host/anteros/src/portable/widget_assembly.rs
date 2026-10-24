//! Bounded textual assembly and deterministic disassembly for Widget IR.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::{self, Write};

use crate::{
    CallbackBinding, EventId, ModelError, ModelLimits, PropertyId, PropertyValue, StableId128,
    Version, WIDGET_IR_FORMAT_VERSION, WidgetDocument, WidgetDocumentView, WidgetNode,
    WidgetProperty, WidgetSchemaId, stable_schema_hash64,
};

const ASSEMBLY_SOURCE_EXPANSION: usize = 16;
const ASSEMBLY_SOURCE_OVERHEAD: usize = 4_096;

/// The category of a textual Widget IR assembly failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssemblyErrorKind {
    /// A directive or token sequence is malformed.
    InvalidSyntax,
    /// A directive is not part of the assembly grammar in its current section.
    UnknownDirective,
    /// A symbolic node or data label was declared more than once.
    DuplicateLabel,
    /// A symbolic node or data reference has no declaration.
    MissingReference,
    /// The text section has no `ROOT` directive.
    MissingRoot,
    /// A node was not terminated by `END`.
    MissingEnd,
    /// A required `SECTION TEXT` or `SECTION DATA` directive is absent.
    MissingSection,
    /// The assembly declares an unsupported AWIR format version.
    UnsupportedVersion,
    /// A floating-point property is NaN or infinite.
    NonFiniteFloat,
    /// A quoted UTF-8 string contains an invalid escape sequence.
    InvalidEscape,
    /// A blob or fixed identity contains invalid hexadecimal text.
    InvalidHex,
    /// A key or callback identity is not exactly 16 bytes.
    InvalidIdentity,
    /// The source or assembled model exceeds a caller-provided resource limit.
    LimitExceeded,
    /// Canonical Widget IR model validation rejected the assembled document.
    Model(ModelError),
}

/// An error produced while parsing or encoding textual Widget IR assembly.
///
/// Parse failures retain a one-based source line when a single directive caused
/// the error. [`Self::context`] contains the rejected label, token, or directive
/// where practical and never borrows the caller's source text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WidgetAssemblyError {
    kind: AssemblyErrorKind,
    line: Option<usize>,
    context: String,
}

impl WidgetAssemblyError {
    /// Returns the stable failure category.
    #[inline]
    pub const fn kind(&self) -> &AssemblyErrorKind {
        &self.kind
    }

    /// Returns the one-based source line associated with this failure.
    #[inline]
    pub const fn line(&self) -> Option<usize> {
        self.line
    }

    /// Returns the owned token or directive context captured at failure time.
    #[inline]
    pub fn context(&self) -> &str {
        &self.context
    }

    fn at(kind: AssemblyErrorKind, line: usize, context: impl Into<String>) -> Self {
        Self {
            kind,
            line: Some(line),
            context: context.into(),
        }
    }

    fn model(error: ModelError) -> Self {
        let kind = if is_limit_error(error) {
            AssemblyErrorKind::LimitExceeded
        } else {
            AssemblyErrorKind::Model(error)
        };
        Self {
            kind,
            line: None,
            context: error.to_string(),
        }
    }
}

impl fmt::Display for WidgetAssemblyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) if self.context.is_empty() => {
                write!(formatter, "assembly error on line {line}: {:?}", self.kind)
            }
            Some(line) => write!(
                formatter,
                "assembly error on line {line}: {:?}: {}",
                self.kind, self.context
            ),
            None if self.context.is_empty() => write!(formatter, "assembly error: {:?}", self.kind),
            None => write!(
                formatter,
                "assembly error: {:?}: {}",
                self.kind, self.context
            ),
        }
    }
}

impl Error for WidgetAssemblyError {}

/// A bounded, owned textual Widget IR document ready for canonical encoding.
///
/// Parsing resolves all symbolic labels, validates syntax and resource limits,
/// and performs canonical binary encode-and-decode validation before returning.
/// Generation and revision default to zero when their directives are omitted.
#[derive(Debug)]
pub struct WidgetAssemblyDocument {
    generation_id: u64,
    document_revision: u64,
    root_node: u32,
    nodes: Vec<OwnedNode>,
    strings: Vec<String>,
    blobs: Vec<Vec<u8>>,
    limits: ModelLimits,
}

impl WidgetAssemblyDocument {
    /// Parses one owned AWIR 2.0 assembly document under explicit model limits.
    ///
    /// Node, string, and blob labels share one namespace. Stable widget,
    /// property, and event IDs accept either fixed `0x` hexadecimal values or
    /// `hash64("canonical name")` expressions. Symbolic references are resolved
    /// to checked `u32` table indices before this function returns.
    ///
    /// Source text is bounded independently from the binary document ceiling.
    /// The finite budget is sixteen times `max_document_bytes` plus 4 KiB,
    /// using saturating arithmetic. This accommodates the deterministic textual
    /// expansion of every binary record while preserving a hard input bound.
    pub fn parse(source: &str, limits: ModelLimits) -> Result<Self, WidgetAssemblyError> {
        let source_budget = (limits.max_document_bytes as usize)
            .saturating_mul(ASSEMBLY_SOURCE_EXPANSION)
            .saturating_add(ASSEMBLY_SOURCE_OVERHEAD);
        if source.len() > source_budget {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::LimitExceeded,
                1,
                "assembly source exceeds derived source budget",
            ));
        }
        let mut parser = Parser::new(limits);
        for (offset, source_line) in source.lines().enumerate() {
            parser.parse_line(offset + 1, source_line)?;
        }
        let document = parser.finish()?;
        document.encode()?;
        Ok(document)
    }

    /// Encodes this assembly as canonical binary AWIR.
    ///
    /// Encoding first uses [`WidgetDocument::encode`] and then validates the
    /// resulting canonical image with [`WidgetDocumentView::decode`]. Graph
    /// topology, table references, finite values, and all binary resource limits
    /// therefore use the same validation path as received Widget IR documents.
    pub fn encode(&self) -> Result<Vec<u8>, WidgetAssemblyError> {
        let nodes = self
            .nodes
            .iter()
            .map(|node| {
                let mut encoded = WidgetNode::new(node.widget_type, node.widget_schema)
                    .properties(&node.properties)
                    .callbacks(&node.callbacks)
                    .children(&node.children);
                if let Some(key) = node.key {
                    encoded = encoded.key(key);
                }
                encoded
            })
            .collect::<Vec<_>>();
        let strings = self.strings.iter().map(String::as_str).collect::<Vec<_>>();
        let blobs = self.blobs.iter().map(Vec::as_slice).collect::<Vec<_>>();
        let image = WidgetDocument::new(
            self.generation_id,
            self.document_revision,
            self.root_node,
            &nodes,
            &strings,
            &blobs,
        )
        .encode(self.limits)
        .map_err(WidgetAssemblyError::model)?;
        WidgetDocumentView::decode(&image, self.limits).map_err(WidgetAssemblyError::model)?;
        Ok(image)
    }
}

/// Produces deterministic textual assembly from a validated Widget IR view.
///
/// The output uses table-order labels (`nodeN`, `stringN`, and `blobN`), fixed
/// lowercase hexadecimal identities, exact hexadecimal `F64` bits, explicit
/// generation and revision directives, and escaped UTF-8 strings. Parsing the
/// result and encoding it under limits that accept the original image
/// reproduces identical bytes.
pub fn disassemble_widget_document(document: &WidgetDocumentView<'_>) -> String {
    let mut output = String::new();
    writeln!(
        output,
        "AWIR {} {}",
        WIDGET_IR_FORMAT_VERSION.major(),
        WIDGET_IR_FORMAT_VERSION.minor()
    )
    .unwrap();
    writeln!(output, "GENERATION {}", document.generation_id()).unwrap();
    writeln!(output, "REVISION {}", document.document_revision()).unwrap();
    writeln!(output, "SECTION TEXT").unwrap();
    writeln!(output, "ROOT node{}", document.root_node()).unwrap();
    for node_index in 0..document.node_count() {
        let node = document.node(node_index).unwrap();
        writeln!(output, "node{node_index}:").unwrap();
        writeln!(
            output,
            "  NODE 0x{:016x} {} {}",
            node.widget_type().value(),
            node.widget_schema().major(),
            node.widget_schema().minor()
        )
        .unwrap();
        if let Some(key) = node.key() {
            output.push_str("  KEY ");
            write_hex(&mut output, key.as_bytes());
            output.push('\n');
        }
        for property in node.properties() {
            output.push_str("  PROP ");
            if property.is_optional() {
                output.push_str("OPTIONAL ");
            }
            write!(output, "0x{:016x} ", property.property_id().value()).unwrap();
            write_property_value(&mut output, property.value());
            output.push('\n');
        }
        for callback in node.callbacks() {
            write!(
                output,
                "  CALLBACK 0x{:016x} {} {} ",
                callback.event_kind().value(),
                callback.event_schema().major(),
                callback.event_schema().minor()
            )
            .unwrap();
            write_hex(&mut output, callback.callback_id().as_bytes());
            output.push('\n');
        }
        for child in node.children() {
            writeln!(output, "  CHILD node{child}").unwrap();
        }
        writeln!(output, "  END").unwrap();
    }
    writeln!(output, "SECTION DATA").unwrap();
    let mut index = 0_u32;
    while let Some(value) = document.string(index) {
        writeln!(output, "string{index}:").unwrap();
        output.push_str("  STRING \"");
        write_escaped_string(&mut output, value);
        output.push_str("\"\n");
        index += 1;
    }
    index = 0;
    while let Some(value) = document.blob(index) {
        writeln!(output, "blob{index}:").unwrap();
        output.push_str("  BLOB ");
        write_hex(&mut output, value);
        output.push('\n');
        index += 1;
    }
    output
}

#[derive(Debug)]
struct OwnedNode {
    widget_type: WidgetSchemaId,
    widget_schema: Version,
    key: Option<StableId128>,
    properties: Vec<WidgetProperty>,
    callbacks: Vec<CallbackBinding>,
    children: Vec<u32>,
}

struct RawNode {
    widget_type: WidgetSchemaId,
    widget_schema: Version,
    key: Option<StableId128>,
    properties: Vec<RawProperty>,
    callbacks: Vec<CallbackBinding>,
    children: Vec<Reference>,
}

struct RawProperty {
    property_id: PropertyId,
    value: RawValue,
    optional: bool,
}

enum RawValue {
    Direct(PropertyValue),
    StringRef(Reference),
    BlobRef(Reference),
}

struct Reference {
    label: String,
    line: usize,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Section {
    Header,
    Text,
    Data,
}

#[derive(Clone, Copy)]
enum DataIndex {
    String(u32),
    Blob(u32),
}

struct Parser {
    limits: ModelLimits,
    saw_header: bool,
    saw_text: bool,
    saw_data: bool,
    section: Section,
    generation_id: u64,
    document_revision: u64,
    saw_generation: bool,
    saw_revision: bool,
    root: Option<Reference>,
    labels: HashSet<String>,
    node_labels: HashMap<String, u32>,
    data_labels: HashMap<String, DataIndex>,
    pending_label: Option<(String, usize)>,
    current_node: Option<usize>,
    nodes: Vec<RawNode>,
    strings: Vec<String>,
    blobs: Vec<Vec<u8>>,
}

impl Parser {
    fn new(limits: ModelLimits) -> Self {
        Self {
            limits,
            saw_header: false,
            saw_text: false,
            saw_data: false,
            section: Section::Header,
            generation_id: 0,
            document_revision: 0,
            saw_generation: false,
            saw_revision: false,
            root: None,
            labels: HashSet::new(),
            node_labels: HashMap::new(),
            data_labels: HashMap::new(),
            pending_label: None,
            current_node: None,
            nodes: Vec::new(),
            strings: Vec::new(),
            blobs: Vec::new(),
        }
    }

    fn parse_line(
        &mut self,
        line_number: usize,
        source_line: &str,
    ) -> Result<(), WidgetAssemblyError> {
        let line = source_line.trim();
        if line.is_empty() {
            return Ok(());
        }
        if !self.saw_header {
            return self.parse_header(line_number, line);
        }
        if let Some(label) = line.strip_suffix(':') {
            return self.declare_label(line_number, label);
        }
        let fields =
            split_fields(line).map_err(|kind| WidgetAssemblyError::at(kind, line_number, line))?;
        let directive = fields.first().map(String::as_str).unwrap_or("");
        match directive {
            "GENERATION" => self.parse_generation(line_number, &fields, true),
            "REVISION" => self.parse_generation(line_number, &fields, false),
            "SECTION" => self.parse_section(line_number, &fields),
            "ROOT" => self.parse_root(line_number, &fields),
            "NODE" => self.parse_node(line_number, &fields),
            "KEY" => self.parse_key(line_number, &fields),
            "PROP" | "PROP?" => self.parse_property(line_number, &fields, directive == "PROP?"),
            "OPTIONAL" if fields.get(1).map(String::as_str) == Some("PROP") => {
                let mut property_fields = fields[1..].to_vec();
                property_fields[0] = "PROP".to_owned();
                self.parse_property(line_number, &property_fields, true)
            }
            "CALLBACK" => self.parse_callback(line_number, &fields),
            "CHILD" => self.parse_child(line_number, &fields),
            "END" => self.parse_end(line_number, &fields),
            "STRING" => self.parse_string(line_number, line),
            "BLOB" => self.parse_blob(line_number, line),
            _ => Err(WidgetAssemblyError::at(
                AssemblyErrorKind::UnknownDirective,
                line_number,
                directive,
            )),
        }
    }

    fn parse_header(&mut self, line_number: usize, line: &str) -> Result<(), WidgetAssemblyError> {
        let fields =
            split_fields(line).map_err(|kind| WidgetAssemblyError::at(kind, line_number, line))?;
        if fields.len() != 3 || fields[0] != "AWIR" {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::InvalidSyntax,
                line_number,
                line,
            ));
        }
        let major = parse_u16(&fields[1], line_number)?;
        let minor = parse_u16(&fields[2], line_number)?;
        if Version::new(major, minor) != WIDGET_IR_FORMAT_VERSION {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::UnsupportedVersion,
                line_number,
                format!("{major}.{minor}"),
            ));
        }
        self.saw_header = true;
        Ok(())
    }

    fn parse_generation(
        &mut self,
        line: usize,
        fields: &[String],
        generation: bool,
    ) -> Result<(), WidgetAssemblyError> {
        if self.section != Section::Header || fields.len() != 2 {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::InvalidSyntax,
                line,
                fields.join(" "),
            ));
        }
        let seen = if generation {
            &mut self.saw_generation
        } else {
            &mut self.saw_revision
        };
        if *seen {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::InvalidSyntax,
                line,
                fields[0].clone(),
            ));
        }
        let value = fields[1].parse::<u64>().map_err(|_| {
            WidgetAssemblyError::at(AssemblyErrorKind::InvalidSyntax, line, fields[1].clone())
        })?;
        *seen = true;
        if generation {
            self.generation_id = value;
        } else {
            self.document_revision = value;
        }
        Ok(())
    }

    fn parse_section(&mut self, line: usize, fields: &[String]) -> Result<(), WidgetAssemblyError> {
        if fields.len() != 2 || self.pending_label.is_some() {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::InvalidSyntax,
                line,
                fields.join(" "),
            ));
        }
        if self.current_node.is_some() {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::MissingEnd,
                line,
                fields.join(" "),
            ));
        }
        match fields[1].as_str() {
            "TEXT" if self.section == Section::Header && !self.saw_text => {
                self.section = Section::Text;
                self.saw_text = true;
                Ok(())
            }
            "DATA" if self.section == Section::Text && !self.saw_data => {
                self.section = Section::Data;
                self.saw_data = true;
                Ok(())
            }
            _ => Err(WidgetAssemblyError::at(
                AssemblyErrorKind::UnknownDirective,
                line,
                fields.join(" "),
            )),
        }
    }

    fn parse_root(&mut self, line: usize, fields: &[String]) -> Result<(), WidgetAssemblyError> {
        if self.section != Section::Text
            || fields.len() != 2
            || self.root.is_some()
            || self.current_node.is_some()
        {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::InvalidSyntax,
                line,
                fields.join(" "),
            ));
        }
        validate_label(&fields[1], line)?;
        self.root = Some(Reference {
            label: fields[1].clone(),
            line,
        });
        Ok(())
    }

    fn declare_label(&mut self, line: usize, label: &str) -> Result<(), WidgetAssemblyError> {
        if self.section == Section::Header || label.trim() != label || !is_label(label) {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::InvalidSyntax,
                line,
                label,
            ));
        }
        if self.current_node.is_some() {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::MissingEnd,
                line,
                label,
            ));
        }
        if self.pending_label.is_some() {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::InvalidSyntax,
                line,
                label,
            ));
        }
        if !self.labels.insert(label.to_owned()) {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::DuplicateLabel,
                line,
                label,
            ));
        }
        self.pending_label = Some((label.to_owned(), line));
        Ok(())
    }

    fn parse_node(&mut self, line: usize, fields: &[String]) -> Result<(), WidgetAssemblyError> {
        if self.section != Section::Text || fields.len() != 4 || self.current_node.is_some() {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::InvalidSyntax,
                line,
                fields.join(" "),
            ));
        }
        let (label, _) = self.pending_label.take().ok_or_else(|| {
            WidgetAssemblyError::at(
                AssemblyErrorKind::InvalidSyntax,
                line,
                "NODE requires a label",
            )
        })?;
        check_count(self.nodes.len() + 1, self.limits, line)?;
        let index = u32::try_from(self.nodes.len()).map_err(|_| {
            WidgetAssemblyError::at(AssemblyErrorKind::LimitExceeded, line, "node index")
        })?;
        self.node_labels.insert(label, index);
        self.nodes.push(RawNode {
            widget_type: WidgetSchemaId::new(parse_schema_id(&fields[1], line)?),
            widget_schema: Version::new(parse_u16(&fields[2], line)?, parse_u16(&fields[3], line)?),
            key: None,
            properties: Vec::new(),
            callbacks: Vec::new(),
            children: Vec::new(),
        });
        self.current_node = Some(self.nodes.len() - 1);
        Ok(())
    }

    fn parse_key(&mut self, line: usize, fields: &[String]) -> Result<(), WidgetAssemblyError> {
        if fields.len() != 2 {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::InvalidSyntax,
                line,
                fields.join(" "),
            ));
        }
        let node = self.current_node_mut(line, "KEY")?;
        if node.key.is_some() {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::InvalidSyntax,
                line,
                "duplicate KEY",
            ));
        }
        node.key = Some(parse_identity(&fields[1], line)?);
        Ok(())
    }

    fn parse_property(
        &mut self,
        line: usize,
        fields: &[String],
        mut optional: bool,
    ) -> Result<(), WidgetAssemblyError> {
        let mut offset = 1;
        if fields.get(offset).map(String::as_str) == Some("OPTIONAL") {
            optional = true;
            offset += 1;
        }
        if fields.len() != offset + 3 {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::InvalidSyntax,
                line,
                fields.join(" "),
            ));
        }
        let property_id = PropertyId::new(parse_schema_id(&fields[offset], line)?);
        let value = parse_property_value(&fields[offset + 1], &fields[offset + 2], line)?;
        let limit = self.limits;
        let node = self.current_node_mut(line, "PROP")?;
        check_count(node.properties.len() + 1, limit, line)?;
        node.properties.push(RawProperty {
            property_id,
            value,
            optional,
        });
        Ok(())
    }

    fn parse_callback(
        &mut self,
        line: usize,
        fields: &[String],
    ) -> Result<(), WidgetAssemblyError> {
        if fields.len() != 5 {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::InvalidSyntax,
                line,
                fields.join(" "),
            ));
        }
        let callback = CallbackBinding::new(
            EventId::new(parse_schema_id(&fields[1], line)?),
            Version::new(parse_u16(&fields[2], line)?, parse_u16(&fields[3], line)?),
            parse_identity(&fields[4], line)?,
        );
        let limit = self.limits;
        let node = self.current_node_mut(line, "CALLBACK")?;
        check_count(node.callbacks.len() + 1, limit, line)?;
        node.callbacks.push(callback);
        Ok(())
    }

    fn parse_child(&mut self, line: usize, fields: &[String]) -> Result<(), WidgetAssemblyError> {
        if fields.len() != 2 {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::InvalidSyntax,
                line,
                fields.join(" "),
            ));
        }
        validate_label(&fields[1], line)?;
        let limit = self.limits;
        let node = self.current_node_mut(line, "CHILD")?;
        check_count(node.children.len() + 1, limit, line)?;
        node.children.push(Reference {
            label: fields[1].clone(),
            line,
        });
        Ok(())
    }

    fn parse_end(&mut self, line: usize, fields: &[String]) -> Result<(), WidgetAssemblyError> {
        if fields.len() != 1 || self.section != Section::Text || self.current_node.take().is_none()
        {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::InvalidSyntax,
                line,
                fields.join(" "),
            ));
        }
        Ok(())
    }

    fn parse_string(&mut self, line: usize, source: &str) -> Result<(), WidgetAssemblyError> {
        if self.section != Section::Data {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::UnknownDirective,
                line,
                source,
            ));
        }
        let (label, _) = self.take_data_label(line, "STRING")?;
        let value_source = source.strip_prefix("STRING").unwrap().trim_start();
        let value = parse_quoted_string(value_source, line)?;
        if value.len() > self.limits.max_string_bytes as usize {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::LimitExceeded,
                line,
                label,
            ));
        }
        check_count(self.strings.len() + 1, self.limits, line)?;
        let index = u32::try_from(self.strings.len()).map_err(|_| {
            WidgetAssemblyError::at(AssemblyErrorKind::LimitExceeded, line, "string index")
        })?;
        self.data_labels.insert(label, DataIndex::String(index));
        self.strings.push(value);
        Ok(())
    }

    fn parse_blob(&mut self, line: usize, source: &str) -> Result<(), WidgetAssemblyError> {
        if self.section != Section::Data {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::UnknownDirective,
                line,
                source,
            ));
        }
        let (label, _) = self.take_data_label(line, "BLOB")?;
        let value_source = source.strip_prefix("BLOB").unwrap().trim();
        let value_source = value_source.strip_prefix("0x").unwrap_or(value_source);
        if value_source.len() % 2 != 0 || !value_source.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::InvalidHex,
                line,
                value_source,
            ));
        }
        if value_source.len() / 2 > self.limits.max_blob_bytes as usize {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::LimitExceeded,
                line,
                label,
            ));
        }
        let value = parse_hex(value_source, line, AssemblyErrorKind::InvalidHex)?;
        check_count(self.blobs.len() + 1, self.limits, line)?;
        let index = u32::try_from(self.blobs.len()).map_err(|_| {
            WidgetAssemblyError::at(AssemblyErrorKind::LimitExceeded, line, "blob index")
        })?;
        self.data_labels.insert(label, DataIndex::Blob(index));
        self.blobs.push(value);
        Ok(())
    }

    fn current_node_mut(
        &mut self,
        line: usize,
        directive: &str,
    ) -> Result<&mut RawNode, WidgetAssemblyError> {
        if self.section != Section::Text || self.pending_label.is_some() {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::UnknownDirective,
                line,
                directive,
            ));
        }
        let index = self.current_node.ok_or_else(|| {
            WidgetAssemblyError::at(AssemblyErrorKind::UnknownDirective, line, directive)
        })?;
        Ok(&mut self.nodes[index])
    }

    fn take_data_label(
        &mut self,
        line: usize,
        directive: &str,
    ) -> Result<(String, usize), WidgetAssemblyError> {
        self.pending_label.take().ok_or_else(|| {
            WidgetAssemblyError::at(
                AssemblyErrorKind::InvalidSyntax,
                line,
                format!("{directive} requires a label"),
            )
        })
    }

    fn finish(self) -> Result<WidgetAssemblyDocument, WidgetAssemblyError> {
        if !self.saw_header || !self.saw_text || !self.saw_data {
            return Err(WidgetAssemblyError {
                kind: AssemblyErrorKind::MissingSection,
                line: None,
                context: "SECTION TEXT and SECTION DATA are required".to_owned(),
            });
        }
        if self.current_node.is_some() {
            return Err(WidgetAssemblyError {
                kind: AssemblyErrorKind::MissingEnd,
                line: None,
                context: "unterminated node".to_owned(),
            });
        }
        if let Some((label, line)) = self.pending_label {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::InvalidSyntax,
                line,
                label,
            ));
        }
        let root = self.root.ok_or_else(|| WidgetAssemblyError {
            kind: AssemblyErrorKind::MissingRoot,
            line: None,
            context: "ROOT".to_owned(),
        })?;
        let root_node = resolve_node(&self.node_labels, root)?;
        let mut nodes = Vec::with_capacity(self.nodes.len());
        for raw in self.nodes {
            let mut properties = Vec::with_capacity(raw.properties.len());
            for property in raw.properties {
                let optional = property.optional;
                let value = match property.value {
                    RawValue::Direct(value) => value,
                    RawValue::StringRef(reference) => {
                        PropertyValue::StringRef(resolve_data(&self.data_labels, reference, true)?)
                    }
                    RawValue::BlobRef(reference) => {
                        PropertyValue::BlobRef(resolve_data(&self.data_labels, reference, false)?)
                    }
                };
                let property = WidgetProperty::new(property.property_id, value);
                properties.push(if optional {
                    property.optional()
                } else {
                    property
                });
            }
            let children = raw
                .children
                .into_iter()
                .map(|reference| resolve_node(&self.node_labels, reference))
                .collect::<Result<Vec<_>, _>>()?;
            nodes.push(OwnedNode {
                widget_type: raw.widget_type,
                widget_schema: raw.widget_schema,
                key: raw.key,
                properties,
                callbacks: raw.callbacks,
                children,
            });
        }
        Ok(WidgetAssemblyDocument {
            generation_id: self.generation_id,
            document_revision: self.document_revision,
            root_node,
            nodes,
            strings: self.strings,
            blobs: self.blobs,
            limits: self.limits,
        })
    }
}

fn resolve_node(
    labels: &HashMap<String, u32>,
    reference: Reference,
) -> Result<u32, WidgetAssemblyError> {
    labels.get(&reference.label).copied().ok_or_else(|| {
        WidgetAssemblyError::at(
            AssemblyErrorKind::MissingReference,
            reference.line,
            reference.label,
        )
    })
}

fn resolve_data(
    labels: &HashMap<String, DataIndex>,
    reference: Reference,
    string: bool,
) -> Result<u32, WidgetAssemblyError> {
    let value = labels.get(&reference.label).copied();
    match (value, string) {
        (Some(DataIndex::String(index)), true) | (Some(DataIndex::Blob(index)), false) => Ok(index),
        _ => Err(WidgetAssemblyError::at(
            AssemblyErrorKind::MissingReference,
            reference.line,
            reference.label,
        )),
    }
}

fn parse_property_value(
    kind: &str,
    value: &str,
    line: usize,
) -> Result<RawValue, WidgetAssemblyError> {
    let invalid = || WidgetAssemblyError::at(AssemblyErrorKind::InvalidSyntax, line, value);
    match kind {
        "BOOL" => match value {
            "true" | "TRUE" | "1" => Ok(RawValue::Direct(PropertyValue::Bool(true))),
            "false" | "FALSE" | "0" => Ok(RawValue::Direct(PropertyValue::Bool(false))),
            _ => Err(invalid()),
        },
        "I64" => value
            .parse::<i64>()
            .map(|value| RawValue::Direct(PropertyValue::I64(value)))
            .map_err(|_| invalid()),
        "F64" => {
            let parsed = if let Some(bits) = value.strip_prefix("0x") {
                if bits.len() != 16 || !bits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Err(WidgetAssemblyError::at(
                        AssemblyErrorKind::InvalidHex,
                        line,
                        value,
                    ));
                }
                f64::from_bits(u64::from_str_radix(bits, 16).map_err(|_| invalid())?)
            } else {
                value.parse::<f64>().map_err(|_| invalid())?
            };
            if !parsed.is_finite() {
                return Err(WidgetAssemblyError::at(
                    AssemblyErrorKind::NonFiniteFloat,
                    line,
                    value,
                ));
            }
            Ok(RawValue::Direct(PropertyValue::F64(parsed)))
        }
        "RGBA" => parse_fixed_hex_u32(value, line)
            .map(|value| RawValue::Direct(PropertyValue::Rgba(value))),
        "STRREF" => {
            validate_label(value, line)?;
            Ok(RawValue::StringRef(Reference {
                label: value.to_owned(),
                line,
            }))
        }
        "BLOBREF" => {
            validate_label(value, line)?;
            Ok(RawValue::BlobRef(Reference {
                label: value.to_owned(),
                line,
            }))
        }
        _ => Err(WidgetAssemblyError::at(
            AssemblyErrorKind::UnknownDirective,
            line,
            kind,
        )),
    }
}

fn parse_schema_id(value: &str, line: usize) -> Result<u64, WidgetAssemblyError> {
    if let Some(hex) = value.strip_prefix("0x") {
        if hex.len() != 16 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(WidgetAssemblyError::at(
                AssemblyErrorKind::InvalidHex,
                line,
                value,
            ));
        }
        return u64::from_str_radix(hex, 16)
            .map_err(|_| WidgetAssemblyError::at(AssemblyErrorKind::InvalidHex, line, value));
    }
    let inner = value
        .strip_prefix("hash64(")
        .and_then(|value| value.strip_suffix(')'))
        .ok_or_else(|| WidgetAssemblyError::at(AssemblyErrorKind::InvalidSyntax, line, value))?;
    Ok(stable_schema_hash64(&parse_quoted_string(inner, line)?))
}

fn parse_identity(value: &str, line: usize) -> Result<StableId128, WidgetAssemblyError> {
    let hex = value.strip_prefix("0x").unwrap_or(value);
    if hex.len() != 32 {
        return Err(WidgetAssemblyError::at(
            AssemblyErrorKind::InvalidIdentity,
            line,
            value,
        ));
    }
    let bytes = parse_hex(hex, line, AssemblyErrorKind::InvalidHex)?;
    let identity = <[u8; 16]>::try_from(bytes)
        .map_err(|_| WidgetAssemblyError::at(AssemblyErrorKind::InvalidIdentity, line, value))?;
    Ok(StableId128::from_bytes(identity))
}

fn parse_fixed_hex_u32(value: &str, line: usize) -> Result<u32, WidgetAssemblyError> {
    let hex = value
        .strip_prefix("0x")
        .ok_or_else(|| WidgetAssemblyError::at(AssemblyErrorKind::InvalidHex, line, value))?;
    if hex.len() != 8 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(WidgetAssemblyError::at(
            AssemblyErrorKind::InvalidHex,
            line,
            value,
        ));
    }
    u32::from_str_radix(hex, 16)
        .map_err(|_| WidgetAssemblyError::at(AssemblyErrorKind::InvalidHex, line, value))
}

fn parse_hex(
    value: &str,
    line: usize,
    kind: AssemblyErrorKind,
) -> Result<Vec<u8>, WidgetAssemblyError> {
    if value.len() % 2 != 0 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(WidgetAssemblyError::at(kind, line, value));
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for offset in (0..value.len()).step_by(2) {
        bytes.push(
            u8::from_str_radix(&value[offset..offset + 2], 16)
                .map_err(|_| WidgetAssemblyError::at(AssemblyErrorKind::InvalidHex, line, value))?,
        );
    }
    Ok(bytes)
}

fn parse_quoted_string(value: &str, line: usize) -> Result<String, WidgetAssemblyError> {
    let inner = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .ok_or_else(|| WidgetAssemblyError::at(AssemblyErrorKind::InvalidSyntax, line, value))?;
    let mut output = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            output.push(character);
            continue;
        }
        let escaped = chars.next().ok_or_else(|| {
            WidgetAssemblyError::at(AssemblyErrorKind::InvalidEscape, line, value)
        })?;
        match escaped {
            '\\' => output.push('\\'),
            '"' => output.push('"'),
            'n' => output.push('\n'),
            'r' => output.push('\r'),
            't' => output.push('\t'),
            '0' => output.push('\0'),
            'x' => {
                let high = chars.next().and_then(|value| value.to_digit(16));
                let low = chars.next().and_then(|value| value.to_digit(16));
                let byte = high
                    .zip(low)
                    .map(|(high, low)| (high * 16 + low) as u8)
                    .ok_or_else(|| {
                        WidgetAssemblyError::at(AssemblyErrorKind::InvalidEscape, line, value)
                    })?;
                if !byte.is_ascii() {
                    return Err(WidgetAssemblyError::at(
                        AssemblyErrorKind::InvalidEscape,
                        line,
                        value,
                    ));
                }
                output.push(char::from(byte));
            }
            'u' => {
                if chars.next() != Some('{') {
                    return Err(WidgetAssemblyError::at(
                        AssemblyErrorKind::InvalidEscape,
                        line,
                        value,
                    ));
                }
                let mut digits = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(digit) if digit.is_ascii_hexdigit() && digits.len() < 6 => {
                            digits.push(digit)
                        }
                        _ => {
                            return Err(WidgetAssemblyError::at(
                                AssemblyErrorKind::InvalidEscape,
                                line,
                                value,
                            ));
                        }
                    }
                }
                if digits.is_empty() {
                    return Err(WidgetAssemblyError::at(
                        AssemblyErrorKind::InvalidEscape,
                        line,
                        value,
                    ));
                }
                let scalar = u32::from_str_radix(&digits, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(|| {
                        WidgetAssemblyError::at(AssemblyErrorKind::InvalidEscape, line, value)
                    })?;
                output.push(scalar);
            }
            _ => {
                return Err(WidgetAssemblyError::at(
                    AssemblyErrorKind::InvalidEscape,
                    line,
                    value,
                ));
            }
        }
    }
    Ok(output)
}

fn split_fields(line: &str) -> Result<Vec<String>, AssemblyErrorKind> {
    let mut fields = Vec::new();
    let mut start = None;
    let mut quoted = false;
    let mut escaped = false;
    let mut parentheses = 0_u32;
    for (offset, character) in line.char_indices() {
        if start.is_none() {
            if character.is_whitespace() {
                continue;
            }
            start = Some(offset);
        }
        if quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
            continue;
        }
        match character {
            '"' => quoted = true,
            '(' => {
                parentheses = parentheses
                    .checked_add(1)
                    .ok_or(AssemblyErrorKind::InvalidSyntax)?
            }
            ')' => {
                parentheses = parentheses
                    .checked_sub(1)
                    .ok_or(AssemblyErrorKind::InvalidSyntax)?
            }
            _ if character.is_whitespace() && parentheses == 0 => {
                let field_start = start.take().unwrap();
                fields.push(line[field_start..offset].to_owned());
            }
            _ => {}
        }
    }
    if quoted || escaped || parentheses != 0 {
        return Err(AssemblyErrorKind::InvalidSyntax);
    }
    if let Some(field_start) = start {
        fields.push(line[field_start..].to_owned());
    }
    Ok(fields)
}

fn parse_u16(value: &str, line: usize) -> Result<u16, WidgetAssemblyError> {
    value
        .parse::<u16>()
        .map_err(|_| WidgetAssemblyError::at(AssemblyErrorKind::InvalidSyntax, line, value))
}

fn validate_label(value: &str, line: usize) -> Result<(), WidgetAssemblyError> {
    if is_label(value) {
        Ok(())
    } else {
        Err(WidgetAssemblyError::at(
            AssemblyErrorKind::InvalidSyntax,
            line,
            value,
        ))
    }
}

fn is_label(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(byte) if byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

fn check_count(count: usize, limits: ModelLimits, line: usize) -> Result<(), WidgetAssemblyError> {
    if count > limits.max_collection_entries as usize {
        Err(WidgetAssemblyError::at(
            AssemblyErrorKind::LimitExceeded,
            line,
            "max_collection_entries",
        ))
    } else {
        Ok(())
    }
}

fn is_limit_error(error: ModelError) -> bool {
    matches!(
        error,
        ModelError::DocumentTooLarge { .. }
            | ModelError::CollectionTooLarge { .. }
            | ModelError::StringTooLarge { .. }
            | ModelError::BlobTooLarge { .. }
            | ModelError::WidgetDepthExceeded { .. }
            | ModelError::LengthOverflow
    )
}

fn write_property_value(output: &mut String, value: PropertyValue) {
    match value {
        PropertyValue::Bool(value) => write!(output, "BOOL {value}").unwrap(),
        PropertyValue::I64(value) => write!(output, "I64 {value}").unwrap(),
        PropertyValue::F64(value) => write!(output, "F64 0x{:016x}", value.to_bits()).unwrap(),
        PropertyValue::Rgba(value) => write!(output, "RGBA 0x{value:08x}").unwrap(),
        PropertyValue::StringRef(value) => write!(output, "STRREF string{value}").unwrap(),
        PropertyValue::BlobRef(value) => write!(output, "BLOBREF blob{value}").unwrap(),
    }
}

fn write_hex(output: &mut String, bytes: &[u8]) {
    for byte in bytes {
        write!(output, "{byte:02x}").unwrap();
    }
}

fn write_escaped_string(output: &mut String, value: &str) {
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '"' => output.push_str("\\\""),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\0' => output.push_str("\\0"),
            character if character.is_control() => {
                write!(output, "\\u{{{:x}}}", character as u32).unwrap()
            }
            character => output.push(character),
        }
    }
}
