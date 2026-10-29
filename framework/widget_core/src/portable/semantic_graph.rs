use aimer_anteros::{
    CallbackBinding, ModelError, ModelLimits, StableId128, Version, WidgetDocument,
    WidgetDocumentView, WidgetNode, WidgetProperty, WidgetSchemaId, disassemble_widget_document,
};

use super::widget_ir::{OwnedNode, PortableNodeId, PortableWidgetDocument, PortableWidgetLimits};

/// An immutable semantic widget graph produced before binary AWIR encoding.
///
/// Nodes retain typed schema identities, properties, callbacks, and child
/// references. Strings remain owned separately and are addressed by the
/// `StringRef` values in node properties; opaque values use the same indexed
/// ownership model through `BlobRef`. The graph contains no target-native widget
/// or renderer objects.
#[doc(hidden)]
pub struct PortableSemanticGraph {
    generation_id: u64,
    document_revision: u64,
    root: PortableNodeId,
    limits: PortableWidgetLimits,
    nodes: Vec<OwnedNode>,
    strings: Vec<String>,
    blobs: Vec<Vec<u8>>,
}

impl PortableSemanticGraph {
    pub(super) fn new(
        generation_id: u64,
        document_revision: u64,
        root: PortableNodeId,
        limits: PortableWidgetLimits,
        nodes: Vec<OwnedNode>,
        strings: Vec<String>,
        blobs: Vec<Vec<u8>>,
    ) -> Self {
        Self { generation_id, document_revision, root, limits, nodes, strings, blobs }
    }

    /// Returns the guest generation that produced this graph.
    #[inline]
    pub const fn generation_id(&self) -> u64 { self.generation_id }

    /// Returns the monotonically advancing document revision.
    #[inline]
    pub const fn document_revision(&self) -> u64 { self.document_revision }

    /// Returns the document-local root node identity.
    #[inline]
    pub const fn root(&self) -> PortableNodeId { self.root }

    /// Returns the number of semantic nodes.
    #[inline]
    pub fn node_count(&self) -> usize { self.nodes.len() }

    /// Returns the number of interned UTF-8 strings.
    #[inline]
    pub fn string_count(&self) -> usize { self.strings.len() }

    /// Returns one semantic node when its document-local identity exists.
    #[inline]
    pub fn node(&self, id: PortableNodeId) -> Option<PortableSemanticNodeView<'_>> {
        self.nodes
            .get(id.index() as usize)
            .map(|node| PortableSemanticNodeView { node })
    }

    /// Resolves one interned UTF-8 string.
    #[inline]
    pub fn string(&self, index: u32) -> Option<&str> {
        self.strings.get(index as usize).map(String::as_str)
    }

    /// Returns the number of interned opaque blobs.
    #[inline]
    pub fn blob_count(&self) -> usize { self.blobs.len() }

    /// Resolves one interned opaque blob.
    #[inline]
    pub fn blob(&self, index: u32) -> Option<&[u8]> {
        self.blobs.get(index as usize).map(Vec::as_slice)
    }

    /// Produces deterministic human-readable AWIR assembly for diagnostics.
    ///
    /// The assembly retains generation, revision, schema, property, callback,
    /// key, child, and interned-string semantics. Production guest transfer
    /// continues to use the bounded binary document returned by [`Self::compile`].
    pub fn to_assembly(&self) -> Result<String, ModelError> {
        let limits = self.model_limits();
        let image = self.with_document(|document| document.encode(limits))?;
        let document = WidgetDocumentView::decode(&image, limits)?;
        Ok(disassemble_widget_document(&document))
    }

    /// Compiles this validated semantic graph into a binary-encodable document.
    #[inline]
    pub fn compile(self) -> PortableWidgetDocument {
        PortableWidgetDocument::from_graph(self)
    }

    pub(super) fn with_document<R>(
        &self,
        callback: impl FnOnce(&WidgetDocument<'_>) -> R,
    ) -> R {
        let nodes: Vec<_> = self
            .nodes
            .iter()
            .map(|node| {
                WidgetNode::new(node.widget_type, node.widget_schema)
                    .key(node.key)
                    .properties(&node.properties)
                    .callbacks(&node.callbacks)
                    .children(&node.children)
            })
            .collect();
        let strings: Vec<_> = self.strings.iter().map(String::as_str).collect();
        let blobs: Vec<_> = self.blobs.iter().map(Vec::as_slice).collect();
        let document = WidgetDocument::new(
            self.generation_id,
            self.document_revision,
            self.root.index(),
            &nodes,
            &strings,
            &blobs,
        );
        callback(&document)
    }

    pub(super) fn model_limits(&self) -> ModelLimits {
        self.limits.model_limits()
    }
}

/// A borrowed semantic view of one widget node.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct PortableSemanticNodeView<'a> {
    node: &'a OwnedNode,
}

impl<'a> PortableSemanticNodeView<'a> {
    /// Returns the stable widget schema identity.
    #[inline]
    pub const fn widget_type(self) -> WidgetSchemaId { self.node.widget_type }

    /// Returns the widget schema version.
    #[inline]
    pub const fn widget_schema(self) -> Version { self.node.widget_schema }

    /// Returns the stable widget-instance identity.
    #[inline]
    pub const fn key(self) -> StableId128 { self.node.key }

    /// Returns the typed properties in deterministic insertion order.
    #[inline]
    pub fn properties(self) -> &'a [WidgetProperty] {
        &self.node.properties
    }

    /// Returns the callback bindings in deterministic insertion order.
    #[inline]
    pub fn callbacks(self) -> &'a [CallbackBinding] {
        &self.node.callbacks
    }

    /// Iterates child identities in declarative order.
    #[inline]
    pub fn children(self) -> impl ExactSizeIterator<Item = PortableNodeId> + 'a {
        self.node.children.iter().copied().map(PortableNodeId::new)
    }
}
