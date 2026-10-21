use scafra_core::GraphNodeKind;

use crate::model::{GraphModel, ModuleNode};
pub(crate) fn render_graph(model: &GraphModel, output: &mut String) {
    output.push_str("\npub mod __scafra_graph {\n");
    output.push_str("    static NODES: &[::scafra::core::GraphNodeDescriptor] = &[\n");
    for node in &model.nodes {
        output.push_str(&format!(
            "        ::scafra::core::GraphNodeDescriptor {{ provider: {}, output: {}, source: {}, kind: {} }},\n",
            rust_string(&node.provider),
            rust_string(&node.output),
            rust_string(&node.source),
            graph_kind_tokens(node.kind)
        ));
    }
    output.push_str("    ];\n");
    output.push_str("    static EDGES: &[::scafra::core::GraphEdgeDescriptor] = &[\n");
    for edge in &model.edges {
        output.push_str(&format!(
            "        ::scafra::core::GraphEdgeDescriptor {{ consumer: {}, dependency: {}, source: {} }},\n",
            rust_string(&edge.consumer),
            rust_string(&edge.dependency),
            rust_string(&edge.source)
        ));
    }
    output.push_str("    ];\n");
    output.push_str(
        "    static DESCRIPTOR: ::scafra::core::GraphDescriptor = ::scafra::core::GraphDescriptor { nodes: NODES, edges: EDGES };\n\n",
    );
    output.push_str(
        "    pub fn descriptor() -> &'static ::scafra::core::GraphDescriptor { &DESCRIPTOR }\n\n",
    );

    let resource_indices = model
        .order
        .iter()
        .copied()
        .filter(|index| model.nodes[*index].asynchronous)
        .collect::<Vec<_>>();
    output.push_str("    pub struct Graph {\n");
    for (index, node) in model.nodes.iter().enumerate() {
        let has_consumers = model
            .edges
            .iter()
            .any(|edge| edge.dependency == node.output);
        if !has_consumers && !node.asynchronous {
            output.push_str(&format!(
                "        pub node_{index}: {},\n",
                graph_output_type(model, node)
            ));
        }
        if node.asynchronous && has_consumers {
            output.push_str(&format!(
                "        managed_{index}: {},\n",
                graph_output_type(model, node)
            ));
        }
        if node.asynchronous && !has_consumers {
            output.push_str(&format!(
                "        managed_{index}: {},\n",
                graph_output_type(model, node)
            ));
        }
    }
    output.push_str("    }\n\n    pub struct Resources {\n");
    for index in &resource_indices {
        output.push_str(&format!(
            "        resource_{index}: {},\n",
            graph_output_type(model, &model.nodes[*index])
        ));
    }
    output.push_str("    }\n\n");
    render_resource_set_impl(model, &resource_indices, output);

    output.push_str("    impl Graph {\n");
    if resource_indices.is_empty() {
        output.push_str("        /// Builds the graph router.\n        pub fn build_router(self, actuator: &::scafra::ActuatorConfig, security: &::scafra::SecurityConfig, request_timeout: Option<::std::time::Duration>) -> Result<::scafra::web::axum::Router, ::scafra::web::WebError> {\n            let (result, _resources) = self.build_router_and_resources(actuator, security, request_timeout);\n            result\n        }\n\n");
    }
    output.push_str("        /// Builds the graph router and returns its application-owned async resources.\n        /// Unknown configured health-check names are returned as startup errors.\n        pub fn build_router_and_resources(self, actuator: &::scafra::ActuatorConfig, security: &::scafra::SecurityConfig, request_timeout: Option<::std::time::Duration>) -> (Result<::scafra::web::axum::Router, ::scafra::web::WebError>, Resources) {\n            match ::scafra::actuator::try_router(actuator) {\n                Ok(router) => self.build_router_with_actuator_router_and_resources(router, actuator, security, request_timeout),\n                Err(error) => { let resources = self.into_resources(); (Err(::scafra::web::WebError::from(error)), resources) }\n            }\n        }\n\n");
    if resource_indices.is_empty() {
        output.push_str("        /// Builds the graph router with checks owned by this application's registry.\n        pub fn build_router_with_health_registry(self, actuator: &::scafra::ActuatorConfig, security: &::scafra::SecurityConfig, health_registry: ::scafra::HealthRegistry, request_timeout: Option<::std::time::Duration>) -> Result<::scafra::web::axum::Router, ::scafra::web::WebError> {\n            let (result, _resources) = self.build_router_with_health_registry_and_resources(actuator, security, health_registry, request_timeout);\n            result\n        }\n\n");
    }
    output.push_str("        /// Builds the router and returns resources retained by this graph.\n        pub fn build_router_with_health_registry_and_resources(self, actuator: &::scafra::ActuatorConfig, security: &::scafra::SecurityConfig, health_registry: ::scafra::HealthRegistry, request_timeout: Option<::std::time::Duration>) -> (Result<::scafra::web::axum::Router, ::scafra::web::WebError>, Resources) {\n            match ::scafra::actuator::router_with_health_registry(actuator, health_registry) {\n                Ok(router) => self.build_router_with_actuator_router_and_resources(router, actuator, security, request_timeout),\n                Err(error) => { let resources = self.into_resources(); (Err(::scafra::web::WebError::from(error)), resources) }\n            }\n        }\n\n        fn into_resources(self) -> Resources {\n            Resources {\n");
    for index in &resource_indices {
        output.push_str(&format!(
            "                resource_{index}: self.managed_{index},\n"
        ));
    }
    output.push_str("            }\n        }\n\n        fn build_router_with_actuator_router_and_resources(self, mut router: ::scafra::web::axum::Router, actuator: &::scafra::ActuatorConfig, security: &::scafra::SecurityConfig, request_timeout: Option<::std::time::Duration>) -> (Result<::scafra::web::axum::Router, ::scafra::web::WebError>, Resources) {\n            let Self {");
    for (index, node) in model.nodes.iter().enumerate() {
        let has_consumers = model
            .edges
            .iter()
            .any(|edge| edge.dependency == node.output);
        if (!has_consumers && !node.asynchronous) || node.asynchronous {
            if node.asynchronous {
                output.push_str(&format!("                managed_{index},\n"));
            } else {
                output.push_str(&format!("                node_{index},\n"));
            }
        }
    }
    output.push_str("            } = self;\n            let resources = Resources {\n");
    for index in &resource_indices {
        output.push_str(&format!(
            "                resource_{index}: managed_{index},\n"
        ));
    }
    output.push_str("            };\n            let route_result = (|| -> Result<::scafra::web::axum::Router, ::scafra::web::WebError> {\n            let mut routes = ::scafra::web::default_controller_metadata(&[");
    for node in &model.nodes {
        if node.kind == GraphNodeKind::Controller
            && !model
                .edges
                .iter()
                .any(|edge| edge.dependency == node.output)
        {
            output.push_str(&format!("{} ,", rust_string(&node.output)));
        }
    }
    output.push_str("]);\n");
    for node in &model.nodes {
        if node.kind == GraphNodeKind::Controller
            && !model
                .edges
                .iter()
                .any(|edge| edge.dependency == node.output)
        {
            output.push_str(&format!(
                "            routes.extend(<{} as ::scafra::web::ControllerRoutes>::route_metadata().iter().copied());\n",
                node.output_type
            ));
        }
    }
    output.push_str("            ::scafra::web::validate_routes(&routes, actuator)?;\n            router = ::scafra::web::register_default_controllers(router, &[");
    for node in &model.nodes {
        if node.kind == GraphNodeKind::Controller
            && !model
                .edges
                .iter()
                .any(|edge| edge.dependency == node.output)
        {
            output.push_str(&format!("{} ,", rust_string(&node.output)));
        }
    }
    output.push_str("]);\n");
    for (index, node) in model.nodes.iter().enumerate() {
        if node.kind == GraphNodeKind::Controller
            && !model
                .edges
                .iter()
                .any(|edge| edge.dependency == node.output)
        {
            output.push_str(&format!(
                "            router = {}::__scafra_register_routes_with(router, node_{index});\n",
                node.output_type
            ));
        }
    }
    output.push_str("            ::scafra::web::finish_router(router, &routes, actuator, security, request_timeout)\n            })();\n            (route_result, resources)\n        }\n    }\n\n");
    output.push_str("    #[derive(Default)]\n    pub struct Overrides {\n");
    for (index, node) in model.nodes.iter().enumerate() {
        output.push_str(&format!(
            "        pub node_{index}: Option<{}>,\n",
            graph_output_type(model, node)
        ));
    }
    output.push_str("    }\n\n");
    if !model.nodes.iter().any(|node| node.asynchronous) {
        output.push_str("    pub fn compose() -> Result<Graph, ::scafra::core::GraphError> { compose_with(Overrides::default()) }\n\n");
        output.push_str("    pub fn compose_with(overrides: Overrides) -> Result<Graph, ::scafra::core::GraphError> { compose_inner(overrides) }\n\n");
        render_compose(model, false, output);
    }
    output.push_str("    pub async fn compose_async() -> Result<Graph, ::scafra::core::GraphError> { compose_async_with(Overrides::default()).await }\n\n");
    output.push_str("    pub async fn compose_async_with(overrides: Overrides) -> Result<Graph, ::scafra::core::GraphError> { compose_async_inner(overrides).await }\n\n");
    render_compose(model, true, output);
    output.push_str("}\n");
}

fn render_resource_set_impl(model: &GraphModel, resource_indices: &[usize], output: &mut String) {
    output.push_str("    impl ::scafra::core::AsyncResourceSet for Resources {\n        fn request_shutdown(&self, reason: ::scafra::ShutdownReason) {\n");
    for index in resource_indices.iter().rev() {
        let node = &model.nodes[*index];
        let target = if graph_node_is_shared(model, node) {
            format!("self.resource_{index}.as_ref()")
        } else {
            format!("&self.resource_{index}")
        };
        output.push_str(&format!(
            "            ::scafra::core::AsyncResource::request_shutdown({target}, reason);\n"
        ));
    }
    output.push_str("        }\n\n        async fn shutdown(&self, reason: ::scafra::ShutdownReason, policy: ::scafra::ShutdownPolicy) -> Result<(), ::scafra::core::GraphError> {\n                let mut failures = Vec::new();\n");
    for index in resource_indices.iter().rev() {
        let node = &model.nodes[*index];
        let target = if graph_node_is_shared(model, node) {
            format!("self.resource_{index}.as_ref()")
        } else {
            format!("&self.resource_{index}")
        };
        output.push_str(&format!(
            "                ::scafra::core::AsyncResource::request_shutdown({target}, reason);\n                if let Err(error) = ::scafra::core::AsyncResource::shutdown({target}, reason, policy).await {{ failures.push(::scafra::core::ProviderFailure::cleanup_failure({}, {}, error)); }}\n",
            rust_string(&node.provider),
            rust_string(&node.source)
        ));
    }
    output.push_str("                if failures.is_empty() { Ok(()) } else { Err(::scafra::core::GraphError::shutdown_failure(failures)) }\n        }\n    }\n\n");
}

fn render_compose(model: &GraphModel, asynchronous: bool, output: &mut String) {
    let name = if asynchronous {
        "compose_async_inner"
    } else {
        "compose_inner"
    };
    if asynchronous {
        output.push_str(&format!("    async fn {name}(mut overrides: Overrides) -> Result<Graph, ::scafra::core::GraphError> {{\n            let _plan = descriptor().plan()?;\n"));
    } else {
        output.push_str(&format!("    fn {name}(mut overrides: Overrides) -> Result<Graph, ::scafra::core::GraphError> {{\n        let _plan = descriptor().plan()?;\n"));
    }
    for index in &model.order {
        let node = &model.nodes[*index];
        let arguments = node
            .dependencies
            .iter()
            .map(|dependency| {
                let dependency_index = model
                    .nodes
                    .iter()
                    .position(|candidate| candidate.output == dependency.output)
                    .unwrap_or(0);
                if dependency.shared {
                    format!("node_{dependency_index}.clone()")
                } else {
                    format!("node_{dependency_index}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        let await_call = if asynchronous && node.asynchronous {
            ".await"
        } else {
            ""
        };
        let call = format!("{}({arguments}){await_call}", node.provider_expression);
        let shared = graph_node_is_shared(model, node);
        output.push_str(&format!("            let node_{index} = match overrides.node_{index}.take() {{\n                Some(value) => value,\n                None => {{\n"));
        if node.fallible {
            if !asynchronous {
                let result = if shared {
                    format!("{call}.map(::std::sync::Arc::new)")
                } else {
                    call.clone()
                };
                output.push_str(&format!(
                    "                    {result}.map_err(|error| ::scafra::core::GraphError::provider_failure({}, ::scafra::core::GraphPhase::Construction, {}, error))?\n",
                    rust_string(&node.provider),
                    rust_string(&node.source)
                ));
            } else {
                output.push_str(&format!(
                    "                    let value = match {call} {{ Ok(value) => value, Err(error) => {{\n                        let original = ::scafra::core::ProviderFailure::new({}, ::scafra::core::GraphPhase::Construction, {}, error);\n                        let mut cleanup_failures = Vec::new();\n",
                    rust_string(&node.provider),
                    rust_string(&node.source)
                ));
                let current_position = model
                    .order
                    .iter()
                    .position(|prior| prior == index)
                    .unwrap_or_default();
                for prior in model.order[..current_position].iter().rev() {
                    let prior = *prior;
                    let prior_node = &model.nodes[prior];
                    if !prior_node.asynchronous {
                        continue;
                    }
                    let target = if graph_node_is_shared(model, prior_node) {
                        format!("node_{prior}.as_ref()")
                    } else {
                        format!("&node_{prior}")
                    };
                    output.push_str(&format!(
                        "                        ::scafra::core::AsyncResource::request_shutdown({target}, ::scafra::ShutdownReason::StartupFailure);\n                        if let Err(error) = ::scafra::core::AsyncResource::shutdown({target}, ::scafra::ShutdownReason::StartupFailure, ::scafra::ShutdownPolicy::default()).await {{ cleanup_failures.push(::scafra::core::ProviderFailure::cleanup_failure({}, {}, error)); }}\n",
                        rust_string(&prior_node.provider),
                        rust_string(&prior_node.source)
                    ));
                }
                output.push_str("                        return Err(::scafra::core::GraphError::startup_failure(original, cleanup_failures));\n                    } };\n");
                if shared {
                    output.push_str("                    ::std::sync::Arc::new(value)\n");
                } else {
                    output.push_str("                    value\n");
                }
            }
        } else {
            if shared {
                output.push_str(&format!(
                    "                    ::std::sync::Arc::new({call})\n"
                ));
            } else {
                output.push_str(&format!("                    {call}\n"));
            }
        }
        output.push_str("                }\n            };\n");
    }

    output.push_str(if asynchronous {
        "            Ok(Graph {\n"
    } else {
        "        Ok(Graph {\n"
    });
    for (index, node) in model.nodes.iter().enumerate() {
        let has_consumers = model
            .edges
            .iter()
            .any(|edge| edge.dependency == node.output);
        if !has_consumers && !node.asynchronous {
            let indentation = if asynchronous {
                "                "
            } else {
                "            "
            };
            output.push_str(&format!("{indentation}node_{index},\n"));
        }
        if node.asynchronous {
            let value = if has_consumers {
                format!("node_{index}.clone()")
            } else {
                format!("node_{index}")
            };
            let indentation = if asynchronous {
                "                "
            } else {
                "            "
            };
            output.push_str(&format!("{indentation}managed_{index}: {value},\n"));
        }
    }
    output.push_str(if asynchronous {
        "            })\n    }\n\n"
    } else {
        "        })\n    }\n\n"
    });
}

fn graph_node_is_shared(model: &GraphModel, node: &crate::model::GraphNode) -> bool {
    model
        .edges
        .iter()
        .any(|edge| edge.dependency == node.output && edge.shared)
}

fn graph_output_type(model: &GraphModel, node: &crate::model::GraphNode) -> String {
    if graph_node_is_shared(model, node) {
        format!("::std::sync::Arc<{}>", node.output_type)
    } else {
        node.output_type.clone()
    }
}

fn graph_kind_tokens(kind: GraphNodeKind) -> &'static str {
    match kind {
        GraphNodeKind::Bean => "::scafra::core::GraphNodeKind::Bean",
        GraphNodeKind::Service => "::scafra::core::GraphNodeKind::Service",
        GraphNodeKind::Component => "::scafra::core::GraphNodeKind::Component",
        GraphNodeKind::Repository => "::scafra::core::GraphNodeKind::Repository",
        GraphNodeKind::Controller => "::scafra::core::GraphNodeKind::Controller",
    }
}

fn rust_string(value: &str) -> String {
    format!("{value:?}")
}

pub(crate) fn render_node(node: &ModuleNode, indent: usize, output: &mut String) {
    let padding = "    ".repeat(indent);
    for (module_name, path) in &node.files {
        output.push_str(&format!(
            "{padding}#[path = {:?}]\n{padding}pub mod {module_name};\n",
            path.display().to_string()
        ));
    }
    for (module_name, child) in &node.children {
        output.push_str(&format!("{padding}pub mod {module_name} {{\n"));
        render_node(child, indent + 1, output);
        output.push_str(&format!("{padding}}}\n"));
    }
}
