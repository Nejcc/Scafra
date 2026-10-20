use std::{collections::BTreeMap, io};

use scafra_core::GraphNodeKind;
use syn::{
    spanned::Spanned, Attribute, Fields, FnArg, Item, ItemFn, ItemStruct, PathArguments,
    ReturnType, Type, Visibility,
};

use crate::{
    graph::graph_order,
    model::{GraphDependency, GraphEdge, GraphModel, GraphNode, SourceFile, TypeDeclaration},
};
pub(crate) fn parse_graph(files: &[SourceFile]) -> io::Result<Result<GraphModel, Vec<String>>> {
    parse_graph_inner(files, false)
}

pub(crate) fn parse_application_graph(
    files: &[SourceFile],
) -> io::Result<Result<GraphModel, Vec<String>>> {
    parse_graph_inner(files, true)
}

fn parse_graph_inner(
    files: &[SourceFile],
    skip_private: bool,
) -> io::Result<Result<GraphModel, Vec<String>>> {
    let mut type_paths = BTreeMap::<String, Vec<TypeDeclaration>>::new();
    for file in files {
        for item in &file.syntax.items {
            if let Item::Struct(item) = item {
                if is_public(&item.vis) {
                    type_paths
                        .entry(item.ident.to_string())
                        .or_default()
                        .push(TypeDeclaration {
                            path: type_path(file, &item.ident.to_string()),
                            source: format!(
                                "{}:{}",
                                file.source_label,
                                item.struct_token.span().start().line
                            ),
                        });
                }
            }
        }
    }

    let mut nodes = Vec::new();
    let mut errors = Vec::new();
    for file in files {
        for item in &file.syntax.items {
            let source = declaration_source(&file.source_label, item);
            let result = match item {
                Item::Struct(item) => {
                    parse_struct_node(item, file, &source, &type_paths, skip_private)
                }
                Item::Fn(item) => parse_bean_node(item, file, &source, &type_paths, skip_private),
                _ => Ok(None),
            };
            match result {
                Ok(Some(node)) => nodes.push(node),
                Ok(None) => {}
                Err(error) => errors.push(error),
            }
        }
    }
    if !errors.is_empty() {
        return Ok(Err(errors));
    }

    nodes.sort_by(|left, right| {
        left.output
            .cmp(&right.output)
            .then_with(|| left.provider.cmp(&right.provider))
    });
    let node_names = nodes
        .iter()
        .map(|node| node.output.clone())
        .collect::<Vec<_>>();
    let mut providers = BTreeMap::<String, &GraphNode>::new();
    let mut duplicate_errors = Vec::new();
    for node in &nodes {
        if let Some(first) = providers.insert(node.output.clone(), node) {
            duplicate_errors.push(format!(
                "Scafra graph has duplicate output `{}` from `{}` at {} and `{}` at {}",
                node.output, first.provider, first.source, node.provider, node.source
            ));
        }
    }
    if !duplicate_errors.is_empty() {
        return Ok(Err(duplicate_errors));
    }
    let mut edges = Vec::new();
    for node in &nodes {
        for dependency in &node.dependencies {
            edges.push(GraphEdge {
                consumer: node.output.clone(),
                dependency: dependency.output.clone(),
                source: node.source.clone(),
                shared: dependency.shared,
            });
        }
    }
    edges.sort_by(|left, right| {
        left.consumer
            .cmp(&right.consumer)
            .then_with(|| left.dependency.cmp(&right.dependency))
            .then_with(|| left.source.cmp(&right.source))
    });

    match graph_order(&node_names, &edges) {
        Ok(order) => {
            let resource_ownership_errors = nodes
                .iter()
                .filter(|node| node.asynchronous)
                .flat_map(|node| {
                    edges
                        .iter()
                        .filter(move |edge| edge.dependency == node.output && !edge.shared)
                        .map(move |edge| {
                            format!(
                                "Scafra async bean `{}` at {} must be injected as `Arc<{}>` by `{}` so the application retains ownership for shutdown",
                                node.provider, node.source, node.output, edge.consumer
                            )
                        })
                })
                .collect::<Vec<_>>();
            if !resource_ownership_errors.is_empty() {
                return Ok(Err(resource_ownership_errors));
            }
            Ok(Ok(GraphModel {
                nodes,
                edges,
                order,
            }))
        }
        Err(errors) => Ok(Err(errors)),
    }
}

fn declaration_source(source_label: &str, item: &Item) -> String {
    format!("{source_label}:{}", declaration_line(item))
}

/// Returns the line containing the Rust declaration keyword, rather than the
/// item's ordinal in the parsed file or the line of an outer attribute.
///
/// Graph source labels are shown to application developers in generated
/// reports and diagnostics. Keeping this choice in one helper makes the
/// build-time and generated metadata paths use the same source convention.
fn declaration_line(item: &Item) -> usize {
    match item {
        Item::Struct(item) => item.struct_token.span().start().line,
        Item::Fn(item) => item.sig.fn_token.span().start().line,
        _ => item.span().start().line,
    }
}

fn parse_struct_node(
    item: &ItemStruct,
    file: &SourceFile,
    source: &str,
    type_paths: &BTreeMap<String, Vec<TypeDeclaration>>,
    skip_private: bool,
) -> Result<Option<GraphNode>, String> {
    let Some(kind) = graph_kind(&item.attrs) else {
        return Ok(None);
    };
    if !is_public(&item.vis) {
        if skip_private {
            return Ok(None);
        }
        return Err(format!(
            "Scafra graph declaration `{}` at {source} must be public so generated composition can call it",
            item.ident
        ));
    }
    let dependencies = match &item.fields {
        Fields::Named(fields) => fields
            .named
            .iter()
            .map(|field| {
                graph_dependency_name(&field.ty, type_paths, &item.ident.to_string(), source)
            })
            .collect::<Result<Vec<_>, _>>()?,
        Fields::Unnamed(fields) => fields
            .unnamed
            .iter()
            .map(|field| {
                graph_dependency_name(&field.ty, type_paths, &item.ident.to_string(), source)
            })
            .collect::<Result<Vec<_>, _>>()?,
        Fields::Unit => Vec::new(),
    };
    let output = item.ident.to_string();
    let output_type = type_path(file, &output);
    Ok(Some(GraphNode {
        kind,
        provider: format!("{output_type}::new"),
        output,
        output_type: output_type.clone(),
        source: source.to_owned(),
        dependencies,
        provider_expression: format!("{output_type}::new"),
        fallible: false,
        asynchronous: false,
    }))
}

fn parse_bean_node(
    item: &ItemFn,
    file: &SourceFile,
    source: &str,
    type_paths: &BTreeMap<String, Vec<TypeDeclaration>>,
    skip_private: bool,
) -> Result<Option<GraphNode>, String> {
    if !has_graph_attribute(&item.attrs, "bean") {
        return Ok(None);
    }
    if !is_public(&item.vis) {
        if skip_private {
            return Ok(None);
        }
        return Err(format!(
            "Scafra graph bean `{}` at {source} must be public so generated composition can call it",
            item.sig.ident
        ));
    }
    let asynchronous = item.sig.asyncness.is_some();
    if !item.sig.generics.params.is_empty() {
        return Err(format!(
            "Scafra graph bean `{}` at {source} must not be generic",
            item.sig.ident
        ));
    }
    let dependencies = item
        .sig
        .inputs
        .iter()
        .map(|argument| match argument {
            FnArg::Typed(argument) => graph_dependency_name(
                &argument.ty,
                type_paths,
                &item.sig.ident.to_string(),
                source,
            ),
            FnArg::Receiver(_) => Err(format!(
                "bean providers cannot have a self receiver at {source}"
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let (output, fallible) =
        bean_output(&item.sig.output).map_err(|error| format!("{error} at {source}"))?;
    if let Some(candidates) = type_paths.get(&output) {
        if candidates.len() > 1 {
            return Err(ambiguous_type_error(
                output.clone(),
                &item.sig.ident.to_string(),
                source,
                candidates,
            ));
        }
    }
    let output_type = resolve_type_path(&output, type_paths).ok_or_else(|| {
        format!(
            "Scafra graph bean `{}` at {source} must return a public concrete type declared in application source",
            item.sig.ident
        )
    })?;
    let provider = format!("{}::{}", module_prefix(file), item.sig.ident);
    Ok(Some(GraphNode {
        kind: GraphNodeKind::Bean,
        provider: provider.clone(),
        output,
        output_type,
        source: source.to_owned(),
        dependencies,
        provider_expression: provider,
        fallible,
        asynchronous,
    }))
}

fn graph_kind(attributes: &[Attribute]) -> Option<GraphNodeKind> {
    [
        ("bean", GraphNodeKind::Bean),
        ("service", GraphNodeKind::Service),
        ("component", GraphNodeKind::Component),
        ("repository", GraphNodeKind::Repository),
        ("controller", GraphNodeKind::Controller),
    ]
    .into_iter()
    .find_map(|(name, kind)| has_graph_attribute(attributes, name).then_some(kind))
}

fn has_graph_attribute(attributes: &[Attribute], name: &str) -> bool {
    attributes.iter().any(|attribute| {
        attribute
            .path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == name)
    })
}

fn is_public(visibility: &Visibility) -> bool {
    matches!(visibility, Visibility::Public(_))
}

fn dependency_name(ty: &Type) -> Result<String, String> {
    let Type::Path(path) = ty else {
        return Err(format!(
            "graph dependency `{ty:?}` must be an owned concrete type path"
        ));
    };
    if path.qself.is_some() {
        return Err(format!(
            "graph dependency `{ty:?}` must not use a qualified self type"
        ));
    }
    if path.path.leading_colon.is_some() || path.path.segments.len() != 1 {
        return Err(format!(
            "graph dependency `{ty:?}` must use an unqualified type name; qualified paths are not supported"
        ));
    }
    let Some(segment) = path.path.segments.last() else {
        return Err("graph dependency must name a concrete type".to_owned());
    };
    if !matches!(segment.arguments, PathArguments::None) {
        return Err(format!("graph dependency `{ty:?}` must not be generic"));
    }
    Ok(segment.ident.to_string())
}

fn graph_dependency_name(
    ty: &Type,
    type_paths: &BTreeMap<String, Vec<TypeDeclaration>>,
    declaration: &str,
    source: &str,
) -> Result<GraphDependency, String> {
    let (name, shared) = match ty {
        Type::Path(path) if path.qself.is_none() => {
            let Some(segment) = path.path.segments.last() else {
                return Err(format!(
                    "graph dependency must name a concrete type at {source}"
                ));
            };
            if segment.ident == "Arc" {
                let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
                    return Err(format!(
                        "shared graph dependencies must use `Arc<T>` at {source}"
                    ));
                };
                if arguments.args.len() != 1 {
                    return Err(format!(
                        "shared graph dependencies must use `Arc<T>` at {source}"
                    ));
                }
                let Some(syn::GenericArgument::Type(inner)) = arguments.args.first() else {
                    return Err(format!(
                        "shared graph dependencies must use `Arc<T>` at {source}"
                    ));
                };
                (
                    dependency_name(inner).map_err(|error| format!("{error} at {source}"))?,
                    true,
                )
            } else {
                (
                    dependency_name(ty).map_err(|error| format!("{error} at {source}"))?,
                    false,
                )
            }
        }
        _ => (
            dependency_name(ty).map_err(|error| format!("{error} at {source}"))?,
            false,
        ),
    };
    let Some(candidates) = type_paths.get(&name) else {
        return Ok(GraphDependency {
            output: name,
            shared,
        });
    };
    if candidates.len() > 1 {
        return Err(ambiguous_type_error(name, declaration, source, candidates));
    }
    Ok(GraphDependency {
        output: name,
        shared,
    })
}

fn bean_output(return_type: &ReturnType) -> Result<(String, bool), String> {
    let ReturnType::Type(_, ty) = return_type else {
        return Err("graph bean providers must declare a concrete return type".to_owned());
    };
    if let Type::Path(path) = ty.as_ref() {
        if let Some(segment) = path.path.segments.last() {
            if segment.ident == "Result" {
                let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
                    return Err("fallible graph beans must return Result<Output, Error>".to_owned());
                };
                let types = arguments
                    .args
                    .iter()
                    .filter_map(|argument| match argument {
                        syn::GenericArgument::Type(ty) => Some(ty),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                if types.len() != 2 || arguments.args.len() != 2 {
                    return Err("fallible graph beans must return Result<Output, Error>".to_owned());
                }
                return Ok((dependency_name(types[0])?, true));
            }
        }
    }
    Ok((dependency_name(ty)?, false))
}

fn resolve_type_path(
    name: &str,
    type_paths: &BTreeMap<String, Vec<TypeDeclaration>>,
) -> Option<String> {
    let paths = type_paths.get(name)?;
    (paths.len() == 1).then(|| paths[0].path.clone())
}

fn ambiguous_type_error(
    name: String,
    declaration: &str,
    source: &str,
    candidates: &[TypeDeclaration],
) -> String {
    let mut candidates = candidates
        .iter()
        .map(|candidate| format!("`{}` at {}", candidate.path, candidate.source))
        .collect::<Vec<_>>();
    candidates.sort();
    let candidates = candidates.join(", ");
    format!(
        "Scafra graph declaration `{declaration}` at {source} has ambiguous public graph type `{name}`; candidates: {candidates}"
    )
}

fn type_path(file: &SourceFile, name: &str) -> String {
    format!("{}::{name}", module_prefix(file))
}

fn module_prefix(file: &SourceFile) -> String {
    if file.module_path.is_empty() {
        "crate".to_owned()
    } else {
        format!("crate::{}", file.module_path.join("::"))
    }
}
