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

    output.push_str("    pub struct Graph {\n");
    for (index, node) in model.nodes.iter().enumerate() {
        if model
            .edges
            .iter()
            .any(|edge| edge.dependency == node.output)
        {
            continue;
        }
        output.push_str(&format!(
            "        pub node_{index}: {},\n",
            graph_output_type(model, node)
        ));
    }
    output.push_str("    }\n\n");
    output.push_str("    impl Graph {\n        /// Builds the graph router with legacy process-wide health registrations.\n        pub fn build_router(self, actuator: &::scafra::ActuatorConfig, security: &::scafra::SecurityConfig, request_timeout: Option<::std::time::Duration>) -> Result<::scafra::web::axum::Router, ::scafra::web::WebError> {\n            let router = ::scafra::actuator::router(actuator);\n            self.build_router_with_actuator_router(router, actuator, security, request_timeout)\n        }\n\n        /// Builds the graph router with checks owned by this application's registry.\n        pub fn build_router_with_health_registry(self, actuator: &::scafra::ActuatorConfig, security: &::scafra::SecurityConfig, health_registry: ::scafra::HealthRegistry, request_timeout: Option<::std::time::Duration>) -> Result<::scafra::web::axum::Router, ::scafra::web::WebError> {\n            let router = ::scafra::actuator::router_with_health_registry(actuator, health_registry)?;\n            self.build_router_with_actuator_router(router, actuator, security, request_timeout)\n        }\n\n        fn build_router_with_actuator_router(self, mut router: ::scafra::web::axum::Router, actuator: &::scafra::ActuatorConfig, security: &::scafra::SecurityConfig, request_timeout: Option<::std::time::Duration>) -> Result<::scafra::web::axum::Router, ::scafra::web::WebError> {\n            let mut routes = ::scafra::web::default_controller_metadata(&[");
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
        if node.kind != GraphNodeKind::Controller
            || model
                .edges
                .iter()
                .any(|edge| edge.dependency == node.output)
        {
            continue;
        }
        output.push_str(&format!(
            "            router = {}::__scafra_register_routes_with(router, self.node_{});\n",
            node.output_type, index
        ));
    }
    output.push_str("            ::scafra::web::finish_router(router, &routes, actuator, security, request_timeout)\n        }\n    }\n\n");
    output.push_str("    #[derive(Default)]\n    pub struct Overrides {\n");
    for (index, node) in model.nodes.iter().enumerate() {
        output.push_str(&format!(
            "        pub node_{index}: Option<{}>,\n",
            graph_output_type(model, node)
        ));
    }
    output.push_str("    }\n\n");
    output.push_str(
        "\n    pub fn compose() -> Result<Graph, ::scafra::core::GraphError> { compose_with(Overrides::default()) }\n\n",
    );
    output.push_str(
        "    pub fn compose_with(mut overrides: Overrides) -> Result<Graph, ::scafra::core::GraphError> {\n",
    );
    output.push_str("        let _plan = descriptor().plan()?;\n");

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
                    .expect("validated graph dependency");
                if dependency.shared {
                    format!("node_{dependency_index}.clone()")
                } else {
                    format!("node_{dependency_index}")
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        let call = format!("{}({arguments})", node.provider_expression);
        let shared = graph_node_is_shared(model, node);
        if !node.fallible && node.dependencies.is_empty() {
            if shared {
                output.push_str(&format!(
                    "        let node_{index} = overrides.node_{index}.take().unwrap_or_else(|| ::std::sync::Arc::new({}()));\n",
                    node.provider_expression
                ));
            } else {
                output.push_str(&format!(
                    "        let node_{index} = overrides.node_{index}.take().unwrap_or_else({});\n",
                    node.provider_expression
                ));
            }
            continue;
        }
        output.push_str(&format!(
            "        let node_{index} = match overrides.node_{index}.take() {{\n            Some(value) => value,\n            None => {{\n"
        ));
        if node.fallible {
            let result = if shared {
                format!("{call}.map(::std::sync::Arc::new)")
            } else {
                call
            };
            output.push_str(&format!(
                "                {result}.map_err(|error| ::scafra::core::GraphError::provider_failure({}, ::scafra::core::GraphPhase::Construction, {}, error))?\n",
                rust_string(&node.provider),
                rust_string(&node.source)
            ));
        } else {
            if shared {
                output.push_str(&format!("                ::std::sync::Arc::new({call})\n"));
            } else {
                output.push_str(&format!("                {call}\n"));
            }
        }
        output.push_str("            }\n        };\n");
    }

    output.push_str("        Ok(Graph {\n");
    for index in 0..model.nodes.len() {
        if model
            .edges
            .iter()
            .any(|edge| edge.dependency == model.nodes[index].output)
        {
            continue;
        }
        output.push_str(&format!("            node_{index},\n"));
    }
    output.push_str("        })\n    }\n}\n");
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
