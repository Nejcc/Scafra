use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

use super::*;
use scafra_core::GraphNodeKind;

#[test]
fn sanitizes_file_names_into_rust_identifiers() {
    assert_eq!(sanitize_ident("user-controller"), "user_controller");
    assert_eq!(sanitize_ident("123users"), "_123users");
}

#[test]
fn graph_order_is_deterministic_and_reports_invalid_edges() {
    let names = vec!["Consumer".to_owned(), "Dependency".to_owned()];
    let edges = vec![GraphEdge {
        consumer: "Consumer".to_owned(),
        dependency: "Dependency".to_owned(),
        source: "src/service.rs:1".to_owned(),
        shared: false,
    }];
    assert_eq!(graph_order(&names, &edges).unwrap(), vec![1, 0]);

    let missing = graph_order(
        &names,
        &[GraphEdge {
            consumer: "Consumer".to_owned(),
            dependency: "Missing".to_owned(),
            source: "src/service.rs:1".to_owned(),
            shared: false,
        }],
    )
    .unwrap_err();
    assert!(missing[0].contains("missing dependency `Missing`"));

    let cycle = graph_order(
        &["A".to_owned(), "B".to_owned()],
        &[
            GraphEdge {
                consumer: "A".to_owned(),
                dependency: "B".to_owned(),
                source: "a.rs:1".to_owned(),
                shared: false,
            },
            GraphEdge {
                consumer: "B".to_owned(),
                dependency: "A".to_owned(),
                source: "b.rs:1".to_owned(),
                shared: false,
            },
        ],
    )
    .unwrap_err();
    assert!(cycle[0].contains("dependency cycle: A -> B -> A"));
}

#[test]
fn graph_order_reports_unknown_consumers_and_owned_graph_conflicts() {
    let unknown = graph_order(
        &["Consumer".to_owned()],
        &[GraphEdge {
            consumer: "MissingConsumer".to_owned(),
            dependency: "Consumer".to_owned(),
            source: "src/missing.rs:7".to_owned(),
            shared: false,
        }],
    )
    .unwrap_err();
    assert!(unknown[0].contains("unknown consumer `MissingConsumer`"));
    assert!(unknown[0].contains("src/missing.rs:7"));

    let fan_out = graph_order(
        &["A".to_owned(), "B".to_owned(), "Root".to_owned()],
        &[
            GraphEdge {
                consumer: "A".to_owned(),
                dependency: "Root".to_owned(),
                source: "src/a.rs:1".to_owned(),
                shared: false,
            },
            GraphEdge {
                consumer: "B".to_owned(),
                dependency: "Root".to_owned(),
                source: "src/b.rs:1".to_owned(),
                shared: false,
            },
        ],
    )
    .unwrap_err();
    assert!(fan_out
        .iter()
        .any(|error| error.contains("multiple consumers")));

    let duplicate_edge = graph_order(
        &["Consumer".to_owned(), "Dependency".to_owned()],
        &[
            GraphEdge {
                consumer: "Consumer".to_owned(),
                dependency: "Dependency".to_owned(),
                source: "src/one.rs:1".to_owned(),
                shared: false,
            },
            GraphEdge {
                consumer: "Consumer".to_owned(),
                dependency: "Dependency".to_owned(),
                source: "src/two.rs:2".to_owned(),
                shared: false,
            },
        ],
    )
    .unwrap_err();
    assert!(duplicate_edge
        .iter()
        .any(|error| error.contains("consumed more than once")));
}

#[test]
fn graph_order_is_stable_for_disconnected_and_deep_graphs() {
    let first_names = vec!["Zed".to_owned(), "Alpha".to_owned(), "Middle".to_owned()];
    let second_names = vec!["Middle".to_owned(), "Zed".to_owned(), "Alpha".to_owned()];
    let first = graph_order(&first_names, &[]).unwrap();
    let second = graph_order(&second_names, &[]).unwrap();
    let first_outputs = first
        .iter()
        .map(|index| first_names[*index].as_str())
        .collect::<Vec<_>>();
    let second_outputs = second
        .iter()
        .map(|index| second_names[*index].as_str())
        .collect::<Vec<_>>();
    assert_eq!(first_outputs, ["Alpha", "Middle", "Zed"]);
    assert_eq!(first_outputs, second_outputs);

    let names = (0..256).map(|index| format!("Node{index:03}"));
    let names = names.collect::<Vec<_>>();
    let edges = (1..names.len())
        .map(|index| GraphEdge {
            consumer: names[index].clone(),
            dependency: names[index - 1].clone(),
            source: format!("src/node{index}.rs:1"),
            shared: false,
        })
        .collect::<Vec<_>>();
    let order = graph_order(&names, &edges).unwrap();
    assert_eq!(order.len(), names.len());
    assert!(order
        .iter()
        .enumerate()
        .all(|(position, index)| *index == position));
}

#[test]
fn graph_order_handles_a_medium_chain_within_the_startup_budget() {
    let names = (0..1_024)
        .map(|index| format!("Node{index:04}"))
        .collect::<Vec<_>>();
    let edges = (1..names.len())
        .map(|index| GraphEdge {
            consumer: names[index].clone(),
            dependency: names[index - 1].clone(),
            source: "src/generated.rs:1".to_owned(),
            shared: false,
        })
        .collect::<Vec<_>>();

    let started = std::time::Instant::now();
    let order = graph_order(&names, &edges).unwrap();
    assert_eq!(order.len(), names.len());
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "medium graph planning exceeded the one-second test budget"
    );
}

#[test]
fn graph_generation_has_bounded_small_and_medium_measurements() {
    const SMALL_BUDGET: std::time::Duration = std::time::Duration::from_millis(250);
    const MEDIUM_BUDGET: std::time::Duration = std::time::Duration::from_secs(1);

    for (label, node_count, budget) in [("small", 32, SMALL_BUDGET), ("medium", 256, MEDIUM_BUDGET)]
    {
        let measurement = measure_graph_generation(node_count);
        eprintln!(
                "graph generation {label}: nodes={node_count}, elapsed={elapsed:?}, threshold={budget:?}, generated_bytes={generated_bytes}",
                elapsed = measurement.elapsed,
                generated_bytes = measurement.generated_bytes,
            );
        assert!(
            measurement.elapsed < budget,
            "{label} graph generation exceeded {budget:?}: {:?}",
            measurement.elapsed
        );
    }
}

#[derive(Debug)]
struct GenerationMeasurement {
    elapsed: std::time::Duration,
    generated_bytes: usize,
}

/// Keeps the generation timing evidence bounded and inspectable without
/// adding a benchmark dependency or a runtime performance API.
fn measure_graph_generation(node_count: usize) -> GenerationMeasurement {
    let syntax = syn::parse_file(&performance_source(node_count)).unwrap();
    let file = SourceFile {
        module_path: vec!["generated".to_owned()],
        source_label: "src/generated.rs".to_owned(),
        syntax,
    };
    let started = std::time::Instant::now();
    let graph = parse_graph(&[file]).unwrap().unwrap();
    let mut generated = String::new();
    render_graph(&graph, &mut generated);
    GenerationMeasurement {
        elapsed: started.elapsed(),
        generated_bytes: generated.len(),
    }
}

fn performance_source(node_count: usize) -> String {
    let mut source = String::new();
    for index in 0..node_count {
        if index == 0 {
            source.push_str("#[service]\npub struct Node0000;\n");
        } else {
            source.push_str(&format!(
                "#[service]\npub struct Node{index:04} {{ previous: Node{:04} }}\n",
                index - 1
            ));
        }
    }
    source
}

#[test]
fn bounded_adversarial_graph_inputs_remain_deterministic() {
    const GENERATED_NODE_COUNT: usize = 64;
    let source = performance_source(GENERATED_NODE_COUNT);
    let parse_generated = || {
        parse_graph(&[SourceFile {
            module_path: vec!["generated".to_owned()],
            source_label: "src/generated.rs".to_owned(),
            syntax: syn::parse_file(&source).expect("generated graph source should parse"),
        }])
        .expect("generated graph parsing should complete")
        .expect("generated graph should be valid")
    };

    let first = parse_generated();
    let second = parse_generated();
    let mut first_rendered = String::new();
    let mut second_rendered = String::new();
    render_graph(&first, &mut first_rendered);
    render_graph(&second, &mut second_rendered);
    assert_eq!(first_rendered, second_rendered);
    assert_eq!(first.order, second.order);
    assert_eq!(first.nodes.len(), GENERATED_NODE_COUNT);

    let duplicate_edges = (0..64)
        .map(|index| GraphEdge {
            consumer: "Node0001".to_owned(),
            dependency: "Node0000".to_owned(),
            source: format!("src/duplicate{index}.rs:1"),
            shared: false,
        })
        .collect::<Vec<_>>();
    let duplicate_errors = graph_order(
        &["Node0000".to_owned(), "Node0001".to_owned()],
        &duplicate_edges,
    )
    .expect_err("duplicate-heavy graph should be rejected");
    assert!(duplicate_errors
        .iter()
        .any(|error| error.contains("consumed more than once")));

    let cycle_errors = graph_order(
        &["A".to_owned(), "B".to_owned(), "C".to_owned()],
        &[
            GraphEdge {
                consumer: "A".to_owned(),
                dependency: "B".to_owned(),
                source: "src/cycle_a.rs:3".to_owned(),
                shared: false,
            },
            GraphEdge {
                consumer: "B".to_owned(),
                dependency: "C".to_owned(),
                source: "src/cycle_b.rs:4".to_owned(),
                shared: false,
            },
            GraphEdge {
                consumer: "C".to_owned(),
                dependency: "A".to_owned(),
                source: "src/cycle_c.rs:5".to_owned(),
                shared: false,
            },
        ],
    )
    .expect_err("cyclic graph should be rejected");
    assert!(cycle_errors
        .iter()
        .any(|error| error.contains("dependency cycle")));

    let disconnected = graph_order(
        &["Zed".to_owned(), "Alpha".to_owned(), "Middle".to_owned()],
        &[],
    )
    .expect("disconnected graph should be valid");
    assert_eq!(disconnected, vec![1, 2, 0]);

    for (source, expected) in [
        (
            "pub struct Value; #[bean] pub fn value<T>() -> Value { Value }",
            "must not be generic",
        ),
        (
            "#[service] pub struct Service { dependency: crate::nested::Dependency }",
            "qualified paths are not supported",
        ),
    ] {
        let result = parse_graph(&[SourceFile {
            module_path: vec!["adversarial".to_owned()],
            source_label: "src/adversarial.rs".to_owned(),
            syntax: syn::parse_file(source).expect("adversarial source should parse"),
        }])
        .expect("adversarial source parsing should complete");
        let errors = match result {
            Ok(_) => panic!("unsupported graph declaration unexpectedly parsed"),
            Err(errors) => errors,
        };
        assert!(errors.iter().any(|error| error.contains(expected)));
        assert!(errors
            .iter()
            .any(|error| error.contains("src/adversarial.rs:")));
    }
}

#[test]
fn graph_order_exhaustively_checks_small_owned_graphs() {
    let names = (0..4).map(|index| format!("Node{index}"));
    let names = names.collect::<Vec<_>>();
    let candidates = (0..names.len())
        .flat_map(|consumer| {
            (0..names.len()).filter_map(move |dependency| {
                (consumer != dependency).then_some((consumer, dependency))
            })
        })
        .collect::<Vec<_>>();

    for mask in 0u16..(1u16 << candidates.len()) {
        let edges = candidates
            .iter()
            .enumerate()
            .filter_map(|(bit, (consumer, dependency))| {
                (mask & (1 << bit) != 0).then_some(GraphEdge {
                    consumer: names[*consumer].clone(),
                    dependency: names[*dependency].clone(),
                    source: format!("src/{consumer}_{dependency}.rs:1"),
                    shared: false,
                })
            })
            .collect::<Vec<_>>();
        let mut consumers_by_dependency = BTreeMap::<usize, BTreeSet<usize>>::new();
        for edge in &edges {
            let consumer = names
                .iter()
                .position(|name| name == &edge.consumer)
                .unwrap();
            let dependency = names
                .iter()
                .position(|name| name == &edge.dependency)
                .unwrap();
            consumers_by_dependency
                .entry(dependency)
                .or_default()
                .insert(consumer);
        }
        let owned = consumers_by_dependency
            .values()
            .all(|consumers| consumers.len() <= 1);
        let acyclic = is_acyclic(names.len(), &candidates, mask);
        match (owned, acyclic, graph_order(&names, &edges)) {
            (true, true, Ok(order)) => {
                let positions = order
                    .iter()
                    .enumerate()
                    .map(|(position, index)| (*index, position))
                    .collect::<BTreeMap<_, _>>();
                assert!(edges.iter().all(|edge| {
                    let consumer = names
                        .iter()
                        .position(|name| name == &edge.consumer)
                        .unwrap();
                    let dependency = names
                        .iter()
                        .position(|name| name == &edge.dependency)
                        .unwrap();
                    positions[&dependency] < positions[&consumer]
                }));
            }
            (false, _, Err(errors)) => {
                assert!(errors
                    .iter()
                    .any(|error| error.contains("multiple consumers")));
            }
            (true, false, Err(errors)) => {
                assert!(errors
                    .iter()
                    .any(|error| error.contains("dependency cycle")));
            }
            (owned, acyclic, result) => {
                panic!("unexpected graph result owned={owned} acyclic={acyclic}: {result:?}")
            }
        }
    }
}

fn is_acyclic(node_count: usize, candidates: &[(usize, usize)], mask: u16) -> bool {
    let mut dependency_count = vec![0usize; node_count];
    let mut dependents = vec![Vec::new(); node_count];
    for (bit, (consumer, dependency)) in candidates.iter().enumerate() {
        if mask & (1 << bit) == 0 {
            continue;
        }
        dependency_count[*consumer] += 1;
        dependents[*dependency].push(*consumer);
    }
    let mut ready = dependency_count
        .iter()
        .enumerate()
        .filter_map(|(index, count)| (*count == 0).then_some(index))
        .collect::<Vec<_>>();
    let mut visited = 0;
    while let Some(index) = ready.pop() {
        visited += 1;
        for dependent in &dependents[index] {
            dependency_count[*dependent] -= 1;
            if dependency_count[*dependent] == 0 {
                ready.push(*dependent);
            }
        }
    }
    visited == node_count
}

#[test]
fn graph_parser_recognizes_nested_types_and_fallible_beans() {
    let syntax = syn::parse_file(
        r#"
                pub struct Prefix;
                #[bean]
                pub fn prefix() -> Prefix { Prefix }
                pub struct Suffix;
                pub struct ProviderError;
                #[bean]
                pub fn suffix(prefix: Prefix) -> Result<Suffix, ProviderError> { Ok(Suffix) }
                #[service]
                pub struct Service { suffix: Suffix }
                #[service]
                pub struct App { service: Service }
            "#,
    )
    .unwrap();
    let file = SourceFile {
        module_path: vec!["nested".to_owned(), "providers".to_owned()],
        source_label: "src/nested/providers.rs".to_owned(),
        syntax,
    };
    let graph = parse_graph(&[file]).unwrap().unwrap();
    assert_eq!(graph.order.len(), 4);
    assert_eq!(graph.nodes[0].output, "App");
    assert!(graph
        .nodes
        .iter()
        .any(|node| node.provider.ends_with("::prefix")));
    assert!(graph
        .nodes
        .iter()
        .find(|node| node.output == "Suffix")
        .is_some_and(|node| node.fallible));
    assert_eq!(
        graph
            .nodes
            .iter()
            .find(|node| node.output == "Suffix")
            .map(|node| node.source.as_str()),
        Some("src/nested/providers.rs:8")
    );
}

#[test]
fn graph_parser_uses_declaration_line_for_node_and_edge_source() {
    let syntax = syn::parse_file(
            "#[allow(dead_code)]\n#[service]\npub struct Dependency;\n\n#[allow(dead_code)]\n#[service]\npub struct Consumer { dependency: Dependency }\n",
        )
        .unwrap();
    let file = SourceFile {
        module_path: vec!["fixture".to_owned()],
        source_label: "src/fixture.rs".to_owned(),
        syntax,
    };

    let graph = parse_graph(&[file]).unwrap().unwrap();
    assert_eq!(
        graph
            .nodes
            .iter()
            .find(|node| node.output == "Consumer")
            .map(|node| node.source.as_str()),
        Some("src/fixture.rs:7")
    );
    assert_eq!(
        graph
            .edges
            .iter()
            .find(|edge| edge.consumer == "Consumer")
            .map(|edge| edge.source.as_str()),
        Some("src/fixture.rs:7")
    );
}

#[test]
fn graph_parser_uses_function_declaration_line_after_outer_attributes() {
    let syntax = syn::parse_file(
        "pub struct Value;\n\n#[allow(dead_code)]\n#[bean]\npub fn value() -> Value { Value }\n",
    )
    .unwrap();
    let file = SourceFile {
        module_path: vec!["fixture".to_owned()],
        source_label: "src/fixture.rs".to_owned(),
        syntax,
    };

    let graph = parse_graph(&[file]).unwrap().unwrap();
    assert_eq!(
        graph
            .nodes
            .iter()
            .find(|node| node.output == "Value")
            .map(|node| node.source.as_str()),
        Some("src/fixture.rs:5")
    );
}

#[test]
fn graph_parser_rejects_ambiguous_dependency_names_before_generation() {
    let first = SourceFile {
        module_path: vec!["first".to_owned()],
        source_label: "src/first.rs".to_owned(),
        syntax: syn::parse_file("pub struct Shared;").unwrap(),
    };
    let second = SourceFile {
        module_path: vec!["second".to_owned()],
        source_label: "src/second.rs".to_owned(),
        syntax: syn::parse_file("\n pub struct Shared;").unwrap(),
    };
    let service = SourceFile {
        module_path: vec!["service".to_owned()],
        source_label: "src/service.rs".to_owned(),
        syntax: syn::parse_file(
            "use crate::second::Shared;\n#[service]\npub struct Service { shared: Shared }",
        )
        .unwrap(),
    };

    let errors = match parse_graph(&[first, second, service]).unwrap() {
        Ok(_) => panic!("ambiguous dependency unexpectedly parsed"),
        Err(errors) => errors,
    };
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("ambiguous public graph type `Shared`"));
    assert!(errors[0].contains("`crate::first::Shared` at src/first.rs:1"));
    assert!(errors[0].contains("`crate::second::Shared` at src/second.rs:2"));
    assert!(errors[0].contains("declaration `Service` at src/service.rs:3"));
}

#[test]
fn graph_parser_rejects_ambiguous_bean_dependency_names() {
    let first = SourceFile {
        module_path: vec!["first".to_owned()],
        source_label: "src/first.rs".to_owned(),
        syntax: syn::parse_file("pub struct Shared;").unwrap(),
    };
    let second = SourceFile {
        module_path: vec!["second".to_owned()],
        source_label: "src/second.rs".to_owned(),
        syntax: syn::parse_file("pub struct Shared;").unwrap(),
    };
    let provider = SourceFile {
            module_path: vec!["provider".to_owned()],
            source_label: "src/provider.rs".to_owned(),
            syntax: syn::parse_file(
                "pub struct Produced;\n#[bean]\npub fn produced(shared: Shared) -> Produced { Produced }",
            )
            .unwrap(),
        };

    let errors = match parse_graph(&[first, second, provider]).unwrap() {
        Ok(_) => panic!("ambiguous bean dependency unexpectedly parsed"),
        Err(errors) => errors,
    };
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("ambiguous public graph type `Shared`"));
    assert!(errors[0].contains("declaration `produced` at src/provider.rs:3"));
}

#[test]
fn graph_parser_reports_source_aware_declaration_errors() {
    let cases = [
        (
            r#"#[service] struct Private;"#,
            "must be public so generated composition can call it",
            true,
        ),
        (
            r#"pub struct Value; #[bean] pub async fn value() -> Value { Value }"#,
            "must be synchronous",
            true,
        ),
        (
            r#"pub struct Value; #[bean] pub fn value<T>() -> Value { Value }"#,
            "must not be generic",
            true,
        ),
        (
            r#"#[service] pub struct Service { dependency: Vec<Value> }"#,
            "must not be generic",
            false,
        ),
        (
            r#"#[service] pub struct Service { dependency: crate::other::Value }"#,
            "qualified paths are not supported",
            true,
        ),
        (
            r#"#[bean] pub fn value() -> Unknown { Unknown }"#,
            "must return a public concrete type declared in application source",
            true,
        ),
    ];

    for (source, expected, has_source) in cases {
        let file = SourceFile {
            module_path: vec!["fixture".to_owned()],
            source_label: "src/fixture.rs".to_owned(),
            syntax: syn::parse_file(source).unwrap(),
        };
        let errors = match parse_graph(&[file]).unwrap() {
            Ok(_) => panic!("invalid graph declaration unexpectedly parsed"),
            Err(errors) => errors,
        };
        assert!(
            errors.iter().any(|error| error.contains(expected)),
            "{errors:?}"
        );
        if has_source {
            assert!(errors.iter().any(|error| error.contains("src/fixture.rs:")));
        }
    }
}

#[test]
fn rendered_graph_contains_typed_calls_and_provider_context() {
    let syntax = syn::parse_file(
        r#"
                pub struct Prefix;
                pub struct Suffix;
                pub struct ProviderError;
                #[bean]
                pub fn prefix() -> Prefix { Prefix }
                #[bean]
                pub fn suffix(prefix: Prefix) -> Result<Suffix, ProviderError> { Ok(Suffix) }
            "#,
    )
    .unwrap();
    let file = SourceFile {
        module_path: vec!["providers".to_owned()],
        source_label: "src/providers.rs".to_owned(),
        syntax,
    };
    let graph = parse_graph(&[file]).unwrap().unwrap();
    let mut generated = String::new();
    render_graph(&graph, &mut generated);
    assert!(generated.contains("pub mod __scafra_graph"));
    assert!(generated.contains("unwrap_or_else(crate::providers::prefix)"));
    assert!(generated.contains("crate::providers::suffix(node_0)"));
    assert!(generated.contains("GraphPhase::Construction"));
    assert!(generated.contains("src/providers.rs:8"));
}

#[test]
fn rendered_graph_composes_services_into_injected_controllers() {
    let syntax = syn::parse_file(
        r#"
            pub struct Greeting;
            #[bean]
            pub fn greeting() -> Greeting { Greeting }
            #[service]
            pub struct GreetingService { greeting: Greeting }
            #[controller("/hello")]
            pub struct GreetingController { service: GreetingService }
        "#,
    )
    .unwrap();
    let file = SourceFile {
        module_path: vec!["app".to_owned()],
        source_label: "src/main/app.rs".to_owned(),
        syntax,
    };
    let graph = parse_graph(&[file]).unwrap().unwrap();
    assert!(graph
        .nodes
        .iter()
        .any(|node| node.kind == GraphNodeKind::Controller));
    let mut generated = String::new();
    render_graph(&graph, &mut generated);
    assert!(generated.contains("crate::app::GreetingService::new(node_"));
    assert!(generated.contains("__scafra_register_routes_with(router, self.node_"));
    assert!(generated.contains("ControllerRoutes>::route_metadata()"));
    assert!(generated.contains("pub fn build_router_with_health_registry"));
    assert!(generated.contains("router_with_health_registry(actuator, health_registry)?"));
}

#[test]
fn rendered_graph_shares_arc_dependencies_and_supports_non_default_consumers() {
    let syntax = syn::parse_file(
        r#"
            use std::sync::Arc;
            pub struct Shared;
            #[bean]
            pub fn shared() -> Shared { Shared }
            #[service]
            pub struct First { shared: Arc<Shared> }
            #[controller("/")]
            pub struct FirstController { service: Arc<First> }
            #[controller("/")]
            pub struct SecondController { service: Arc<First> }
        "#,
    )
    .unwrap();
    let file = SourceFile {
        module_path: vec!["app".to_owned()],
        source_label: "src/main/app.rs".to_owned(),
        syntax,
    };
    let graph = parse_graph(&[file]).unwrap().unwrap();
    assert!(graph
        .edges
        .iter()
        .filter(|edge| edge.dependency == "First")
        .all(|edge| edge.shared));
    let mut generated = String::new();
    render_graph(&graph, &mut generated);
    assert!(generated.contains("Arc<crate::app::First>"));
    assert!(generated.contains("::std::sync::Arc::new(crate::app::First::new(node_3.clone()))"));
    assert!(generated.contains("FirstController::new(node_0.clone())"));
    assert!(generated.contains("__scafra_register_routes_with(router, self.node_"));
}

struct TestBuildExtension {
    metadata: scafra_foundation::PhaseMetadata,
    label: &'static str,
    calls: std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>,
    failure: Option<&'static str>,
    output_path: Option<&'static str>,
}

impl BuildExtension for TestBuildExtension {
    fn metadata(&self) -> scafra_foundation::PhaseMetadata {
        self.metadata
    }

    fn apply(
        &mut self,
        _context: &BuildContext,
        output: &mut BuildOutput,
    ) -> Result<(), BuildExtensionError> {
        self.calls.borrow_mut().push(self.label);
        if let Some(path) = self.output_path {
            let _ = output.emit(path, self.label);
        }
        if let Some(message) = self.failure {
            return Err(BuildExtensionError::new(message));
        }
        Ok(())
    }
}

fn test_context() -> BuildContext {
    BuildContext::new("manifest", "manifest/src", "manifest/target")
}

fn test_extension(
    metadata: scafra_foundation::PhaseMetadata,
    label: &'static str,
    calls: &std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>,
) -> Box<dyn BuildExtension> {
    Box::new(TestBuildExtension {
        metadata,
        label,
        calls: std::rc::Rc::clone(calls),
        failure: None,
        output_path: None,
    })
}

#[test]
fn build_extensions_are_optional_and_run_in_deterministic_order() {
    let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let context = test_context();
    assert_eq!(context.manifest_dir(), std::path::Path::new("manifest"));
    assert_eq!(context.source_root(), std::path::Path::new("manifest/src"));
    assert_eq!(
        context.output_dir(),
        std::path::Path::new("manifest/target")
    );

    let mut output = BuildOutput::new();
    run_extensions(Vec::new(), &context, &mut output).unwrap();
    assert_eq!(output.artifacts().count(), 0);

    let extensions = vec![
        test_extension(
            scafra_foundation::PhaseMetadata::new(
                scafra_foundation::Phase::BuildTime,
                "same",
                10,
                "src/z.rs",
            ),
            "z",
            &calls,
        ),
        test_extension(
            scafra_foundation::PhaseMetadata::new(
                scafra_foundation::Phase::BuildTime,
                "later",
                20,
                "src/later.rs",
            ),
            "later",
            &calls,
        ),
        test_extension(
            scafra_foundation::PhaseMetadata::new(
                scafra_foundation::Phase::BuildTime,
                "same",
                10,
                "src/a.rs",
            ),
            "a",
            &calls,
        ),
    ];
    run_extensions(extensions, &context, &mut output).unwrap();
    assert_eq!(*calls.borrow(), ["a", "z", "later"]);

    let reversed_calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let reversed_extensions = vec![
        test_extension(
            scafra_foundation::PhaseMetadata::new(
                scafra_foundation::Phase::BuildTime,
                "same",
                10,
                "src/a.rs",
            ),
            "a",
            &reversed_calls,
        ),
        test_extension(
            scafra_foundation::PhaseMetadata::new(
                scafra_foundation::Phase::BuildTime,
                "later",
                20,
                "src/later.rs",
            ),
            "later",
            &reversed_calls,
        ),
        test_extension(
            scafra_foundation::PhaseMetadata::new(
                scafra_foundation::Phase::BuildTime,
                "same",
                10,
                "src/z.rs",
            ),
            "z",
            &reversed_calls,
        ),
    ];
    run_extensions(reversed_extensions, &context, &mut BuildOutput::new()).unwrap();
    assert_eq!(*reversed_calls.borrow(), ["a", "z", "later"]);
}

#[test]
fn extension_metadata_is_validated_before_any_extension_runs() {
    let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let invalid = test_extension(
        scafra_foundation::PhaseMetadata::new(
            scafra_foundation::Phase::Runtime,
            "runtime-extension",
            0,
            "src/runtime.rs",
        ),
        "invalid",
        &calls,
    );
    let mut output = BuildOutput::new();
    let error = run_extensions(vec![invalid], &test_context(), &mut output).unwrap_err();
    assert!(matches!(
        error,
        BuildPipelineError::InvalidPhase { metadata }
            if metadata.name() == "runtime-extension"
                && metadata.source() == "src/runtime.rs"
    ));
    assert!(calls.borrow().is_empty());

    let duplicate_calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let duplicate_metadata = scafra_foundation::PhaseMetadata::new(
        scafra_foundation::Phase::BuildTime,
        "duplicate",
        5,
        "src/duplicate.rs",
    );
    let duplicate = vec![
        test_extension(duplicate_metadata, "first", &duplicate_calls),
        test_extension(duplicate_metadata, "second", &duplicate_calls),
    ];
    let error = run_extensions(duplicate, &test_context(), &mut BuildOutput::new()).unwrap_err();
    let message = error.to_string();
    assert!(matches!(
        error,
        BuildPipelineError::DuplicateMetadata { .. }
    ));
    assert_eq!(message.matches("src/duplicate.rs").count(), 2);
    assert!(duplicate_calls.borrow().is_empty());
}

#[test]
fn extension_failures_preserve_metadata_and_controlled_message() {
    let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let extension = Box::new(TestBuildExtension {
        metadata: scafra_foundation::PhaseMetadata::new(
            scafra_foundation::Phase::BuildTime,
            "failing-extension",
            0,
            "build.rs:17",
        ),
        label: "failure",
        calls,
        failure: Some("sanitized extension failure"),
        output_path: None,
    });
    let error =
        run_extensions(vec![extension], &test_context(), &mut BuildOutput::new()).unwrap_err();
    match &error {
        BuildPipelineError::ExtensionFailure { metadata, error } => {
            assert_eq!(metadata.name(), "failing-extension");
            assert_eq!(metadata.source(), "build.rs:17");
            assert_eq!(error.message(), "sanitized extension failure");
        }
        other => panic!("unexpected pipeline error: {other:?}"),
    }
    let message = error.to_string();
    assert!(message.contains("failing-extension"));
    assert!(message.contains("build_time"));
    assert!(message.contains("build.rs:17"));
    assert!(message.contains("sanitized extension failure"));
}

#[test]
fn build_output_rejects_unsafe_and_duplicate_paths_without_mutation() {
    let mut output = BuildOutput::new();
    for path in [
        "",
        ".",
        "../escape",
        "/absolute/artifact",
        "C:/absolute/artifact",
        "nested\\artifact",
        "nested//artifact",
        "nested/./artifact",
    ] {
        assert!(matches!(
            output.emit(path, "contents"),
            Err(BuildOutputError::InvalidPath { .. })
        ));
        assert!(output.artifacts().next().is_none());
    }

    let mut output = BuildOutput::new();
    output.emit("artifact.txt", "first").unwrap();
    assert!(matches!(
        output.emit("artifact.txt", "second"),
        Err(BuildOutputError::DuplicatePath { .. })
    ));
    assert_eq!(output.get("artifact.txt"), Some(b"first".as_slice()));
}

#[test]
fn output_failures_identify_the_extension_and_discovery_does_not_commit() {
    let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let collision_extension = Box::new(TestBuildExtension {
        metadata: scafra_foundation::PhaseMetadata::new(
            scafra_foundation::Phase::BuildTime,
            "generated-collision",
            0,
            "build.rs:19",
        ),
        label: "collision",
        calls: std::rc::Rc::clone(&calls),
        failure: None,
        output_path: Some("scafra_modules.rs"),
    });
    let mut staged_output = BuildOutput::new();
    staged_output
        .emit("scafra_modules.rs", "generated")
        .unwrap();
    let error = run_extensions(
        vec![collision_extension],
        &test_context(),
        &mut staged_output,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        BuildPipelineError::OutputFailure {
            metadata: Some(metadata),
            error: BuildOutputError::DuplicatePath { .. }
        } if metadata.source() == "build.rs:19"
    ));

    let extension = Box::new(TestBuildExtension {
        metadata: scafra_foundation::PhaseMetadata::new(
            scafra_foundation::Phase::BuildTime,
            "unsafe-output",
            0,
            "build.rs:23",
        ),
        label: "unsafe",
        calls: std::rc::Rc::clone(&calls),
        failure: None,
        output_path: Some("../escape.rs"),
    });
    let error =
        run_extensions(vec![extension], &test_context(), &mut BuildOutput::new()).unwrap_err();
    assert!(matches!(
        error,
        BuildPipelineError::OutputFailure {
            metadata: Some(metadata),
            error: BuildOutputError::InvalidPath { .. }
        } if metadata.source() == "build.rs:23"
    ));

    let output_dir = std::env::temp_dir().join(format!(
        "scafra-build-extension-no-commit-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&output_dir).unwrap();
    let generated_path = output_dir.join("scafra_modules.rs");
    let extension = Box::new(TestBuildExtension {
        metadata: scafra_foundation::PhaseMetadata::new(
            scafra_foundation::Phase::BuildTime,
            "failing-extension",
            0,
            "build.rs:31",
        ),
        label: "failure",
        calls,
        failure: Some("stop before commit"),
        output_path: None,
    });
    let result = super::run_discovery_extensions(
        BuildContext::new("manifest", "manifest/src", &output_dir),
        "generated".to_owned(),
        vec![extension],
    );
    assert!(matches!(
        result,
        Err(BuildPipelineError::ExtensionFailure { .. })
    ));
    assert!(!generated_path.exists());
    std::fs::remove_dir_all(output_dir).unwrap();
}

#[test]
fn commit_failures_preserve_destination_path_and_io_source() {
    let outputs = TemporaryOutput::new("commit-failure");
    let destination = outputs.path(GENERATED_MODULES_PATH);
    std::fs::create_dir(&destination).unwrap();
    let expected_source = std::fs::write(&destination, b"generated").unwrap_err();

    let error = super::run_discovery_extensions(
        BuildContext::new("manifest", "manifest/src", outputs.0.clone()),
        "generated".to_owned(),
        Vec::new(),
    )
    .unwrap_err();

    let source = match &error {
        BuildPipelineError::CommitFailure { path, source } => {
            assert_eq!(path, &destination);
            assert_eq!(source.kind(), expected_source.kind());
            source
        }
        other => panic!("unexpected pipeline error: {other:?}"),
    };
    let chained_source = std::error::Error::source(&error)
        .and_then(|source| source.downcast_ref::<std::io::Error>())
        .expect("commit failure should preserve its I/O source");
    assert!(std::ptr::eq(source, chained_source));
    assert_eq!(chained_source.kind(), expected_source.kind());
}

static BUILD_ENVIRONMENT_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

struct BuildEnvironment {
    previous_manifest_dir: Option<OsString>,
    previous_out_dir: Option<OsString>,
}

impl BuildEnvironment {
    fn new(out_dir: &std::path::Path) -> Self {
        let environment = Self {
            previous_manifest_dir: std::env::var_os("CARGO_MANIFEST_DIR"),
            previous_out_dir: std::env::var_os("OUT_DIR"),
        };
        std::env::set_var("CARGO_MANIFEST_DIR", env!("CARGO_MANIFEST_DIR"));
        environment.set_out_dir(out_dir);
        environment
    }

    fn set_out_dir(&self, out_dir: &std::path::Path) {
        std::env::set_var("OUT_DIR", out_dir);
    }
}

impl Drop for BuildEnvironment {
    fn drop(&mut self) {
        match self.previous_manifest_dir.take() {
            Some(value) => std::env::set_var("CARGO_MANIFEST_DIR", value),
            None => std::env::remove_var("CARGO_MANIFEST_DIR"),
        }
        match self.previous_out_dir.take() {
            Some(value) => std::env::set_var("OUT_DIR", value),
            None => std::env::remove_var("OUT_DIR"),
        }
    }
}

struct TemporaryOutput(PathBuf);

impl TemporaryOutput {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "scafra-build-discovery-{label}-{}",
            std::process::id()
        ));
        std::fs::remove_dir_all(&path).ok();
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn opt_in_discovery_preserves_direct_outputs_and_commits_extension_artifacts() {
    let _environment_lock = BUILD_ENVIRONMENT_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap();
    let outputs = TemporaryOutput::new("api-compatibility");
    let direct_modules = outputs.path("direct-modules");
    let direct_graph = outputs.path("direct-graph");
    let opt_in_modules = outputs.path("opt-in-modules");
    let opt_in_graph = outputs.path("opt-in-graph");
    let source_root = outputs.path("source");
    for path in [
        &direct_modules,
        &direct_graph,
        &opt_in_modules,
        &opt_in_graph,
        &source_root,
    ] {
        std::fs::create_dir_all(path).unwrap();
    }
    std::fs::write(source_root.join("main.rs"), "fn main() {}\n").unwrap();

    let environment = BuildEnvironment::new(&direct_modules);
    discover(&source_root).unwrap();
    let direct_modules_source = std::fs::read(direct_modules.join(GENERATED_MODULES_PATH)).unwrap();

    environment.set_out_dir(&direct_graph);
    discover_graph(&source_root).unwrap();
    let direct_graph_source = std::fs::read(direct_graph.join(GENERATED_MODULES_PATH)).unwrap();

    environment.set_out_dir(&opt_in_modules);
    discover_with_extensions(&source_root, Vec::new()).unwrap();
    assert_eq!(
        std::fs::read(opt_in_modules.join(GENERATED_MODULES_PATH)).unwrap(),
        direct_modules_source
    );

    let calls = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let module_extension = Box::new(TestBuildExtension {
        metadata: scafra_foundation::PhaseMetadata::new(
            scafra_foundation::Phase::BuildTime,
            "nested-artifact",
            0,
            "build.rs:41",
        ),
        label: "module-extension",
        calls: std::rc::Rc::clone(&calls),
        failure: None,
        output_path: Some("nested/extension.txt"),
    });
    discover_with_extensions(&source_root, vec![module_extension]).unwrap();
    assert_eq!(
        std::fs::read(opt_in_modules.join(GENERATED_MODULES_PATH)).unwrap(),
        direct_modules_source
    );
    assert_eq!(
        std::fs::read(opt_in_modules.join("nested/extension.txt")).unwrap(),
        b"module-extension"
    );

    environment.set_out_dir(&opt_in_graph);
    discover_graph_with_extensions(&source_root, Vec::new()).unwrap();
    assert_eq!(
        std::fs::read(opt_in_graph.join(GENERATED_MODULES_PATH)).unwrap(),
        direct_graph_source
    );

    assert_eq!(*calls.borrow(), ["module-extension"]);
}
