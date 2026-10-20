use std::{collections::BTreeMap, path::PathBuf};

use scafra_core::GraphNodeKind;

#[derive(Default)]
pub(crate) struct ModuleNode {
    pub(crate) files: BTreeMap<String, PathBuf>,
    pub(crate) children: BTreeMap<String, ModuleNode>,
}

pub(crate) struct SourceFile {
    pub(crate) module_path: Vec<String>,
    pub(crate) source_label: String,
    pub(crate) syntax: syn::File,
}

pub(crate) struct TypeDeclaration {
    pub(crate) path: String,
    pub(crate) source: String,
}

pub(crate) struct GraphNode {
    pub(crate) kind: GraphNodeKind,
    pub(crate) provider: String,
    pub(crate) output: String,
    pub(crate) output_type: String,
    pub(crate) source: String,
    pub(crate) dependencies: Vec<GraphDependency>,
    pub(crate) provider_expression: String,
    pub(crate) fallible: bool,
    pub(crate) asynchronous: bool,
}

pub(crate) struct GraphDependency {
    pub(crate) output: String,
    pub(crate) shared: bool,
}

pub(crate) struct GraphEdge {
    pub(crate) consumer: String,
    pub(crate) dependency: String,
    pub(crate) source: String,
    pub(crate) shared: bool,
}

pub(crate) struct GraphModel {
    pub(crate) nodes: Vec<GraphNode>,
    pub(crate) edges: Vec<GraphEdge>,
    pub(crate) order: Vec<usize>,
}
