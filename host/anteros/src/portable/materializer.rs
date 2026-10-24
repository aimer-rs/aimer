use std::error::Error;
use std::fmt;

use crate::{ModelError, ModelLimits, WidgetDocumentView, WidgetNodeView, WidgetSchemaSupport};

/// Creates one host-owned node from a validated portable widget record.
///
/// A factory is invoked only after the entire binary image, graph topology,
/// resource limits, and widget schemas have passed validation. Children are
/// complete disconnected nodes and retain their document order. Implementors
/// may therefore move them directly into a native widget or element without
/// consulting the live application tree.
pub trait WidgetFactory: WidgetSchemaSupport {
    /// The disconnected host-owned node produced for one Widget IR record.
    type Node;
    /// A factory-defined failure that aborts the complete candidate tree.
    type Error;

    /// Builds one node after all of its children have been built successfully.
    fn build(
        &mut self,
        document: &WidgetDocumentView<'_>,
        node_index: u32,
        node: WidgetNodeView<'_>,
        children: Vec<Self::Node>,
    ) -> Result<Self::Node, Self::Error>;
}

/// Failure while validating or materializing a disconnected widget tree.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WidgetMaterializeError<E> {
    /// The portable image, graph, or widget schema is invalid.
    Model(ModelError),
    /// The host factory registry is invalid before a node can be selected.
    FactorySetup(E),
    /// A host factory rejected one otherwise valid node.
    Factory {
        /// The document-local node whose factory failed.
        node: u32,
        /// The host-defined factory error.
        error: E,
    },
}

impl<E> fmt::Display for WidgetMaterializeError<E>
where
    E: fmt::Display,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Model(error) => write!(formatter, "widget model validation failed: {error}"),
            Self::FactorySetup(error) => write!(formatter, "widget factory setup failed: {error}"),
            Self::Factory { node, error } => {
                write!(formatter, "widget factory failed for node {node}: {error}")
            }
        }
    }
}

impl<E> Error for WidgetMaterializeError<E>
where
    E: Error + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Model(error) => Some(error),
            Self::FactorySetup(error) => Some(error),
            Self::Factory { error, .. } => Some(error),
        }
    }
}

impl<E> From<ModelError> for WidgetMaterializeError<E> {
    #[inline]
    fn from(error: ModelError) -> Self {
        Self::Model(error)
    }
}

/// Validates and materializes one complete disconnected widget tree.
///
/// Validation is atomic with respect to factories: malformed topology,
/// unsupported schemas, and exceeded limits return before [`WidgetFactory::build`]
/// is called. Materialization is iterative and post-order, so children are
/// always complete before their parent. If a factory fails, all nodes already
/// created for the candidate are dropped before this function returns.
pub fn materialize_widget_tree<F>(
    bytes: &[u8],
    limits: ModelLimits,
    factory: &mut F,
) -> Result<F::Node, WidgetMaterializeError<F::Error>>
where
    F: WidgetFactory,
{
    let document = WidgetDocumentView::decode(bytes, limits)?;
    document.validate_schemas(factory)?;

    let node_count = document.node_count() as usize;
    let mut traversal = Vec::with_capacity(node_count.saturating_mul(2));
    let mut post_order = Vec::with_capacity(node_count);
    traversal.push((document.root_node(), false));
    while let Some((node_index, expanded)) = traversal.pop() {
        if expanded {
            post_order.push(node_index);
            continue;
        }
        traversal.push((node_index, true));
        let node = document
            .node(node_index)
            .ok_or(ModelError::NodeIndexOutOfBounds {
                index: node_index,
                node_count: document.node_count(),
            })?;
        for child in node.children().rev() {
            traversal.push((child, false));
        }
    }

    let mut materialized = std::iter::repeat_with(|| None)
        .take(node_count)
        .collect::<Vec<Option<F::Node>>>();
    for node_index in post_order {
        let node = document
            .node(node_index)
            .ok_or(ModelError::NodeIndexOutOfBounds {
                index: node_index,
                node_count: document.node_count(),
            })?;
        let mut children = Vec::with_capacity(node.children().len());
        for child in node.children() {
            children.push(
                materialized[child as usize]
                    .take()
                    .ok_or(ModelError::UnreachableWidgetNode { node: child })?,
            );
        }
        let output = factory
            .build(&document, node_index, node, children)
            .map_err(|error| WidgetMaterializeError::Factory {
                node: node_index,
                error,
            })?;
        materialized[node_index as usize] = Some(output);
    }

    materialized[document.root_node() as usize]
        .take()
        .ok_or(ModelError::UnreachableWidgetNode {
            node: document.root_node(),
        })
        .map_err(Into::into)
}
