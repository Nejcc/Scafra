use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use scafra_actuator::{
    router_with_health_registry, ActuatorConfig, ActuatorSecurity, HealthConfig, HealthRegistry,
    HealthRegistryError,
};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::Duration,
};
use tower::ServiceExt;

fn config(checks: Vec<String>) -> ActuatorConfig {
    ActuatorConfig {
        health: HealthConfig {
            checks,
            ..HealthConfig::default()
        },
        ..ActuatorConfig::all()
    }
}

fn state_registry(name: &str, state: Arc<bool>) -> HealthRegistry {
    let mut registry = HealthRegistry::new();
    let state_for_check = Arc::clone(&state);
    registry
        .register(name, move || {
            let state = Arc::clone(&state_for_check);
            async move { *state }
        })
        .expect("a new registry should accept its first check");
    registry
}

async fn get(router: &Router, path: &str) -> (StatusCode, String) {
    get_with_auth(router, path, None).await
}

async fn get_with_auth(
    router: &Router,
    path: &str,
    authorization: Option<&str>,
) -> (StatusCode, String) {
    let mut request = Request::builder().uri(path);
    if let Some(authorization) = authorization {
        request = request.header("authorization", authorization);
    }
    let response = router
        .clone()
        .oneshot(request.body(Body::empty()).expect("request should build"))
        .await
        .expect("router should respond");
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024)
        .await
        .expect("response body should be readable");
    (
        status,
        String::from_utf8(body.to_vec()).expect("response should be UTF-8"),
    )
}

#[tokio::test]
async fn two_registries_can_use_the_same_name_and_observe_only_their_graph_state() {
    let config = config(vec!["dependency".to_owned()]);
    let registry_a = state_registry("dependency", Arc::new(false));
    let registry_b = state_registry("dependency", Arc::new(true));
    let router_a = router_with_health_registry(&config, registry_a)
        .expect("graph A's configured check should exist");
    let router_b = router_with_health_registry(&config, registry_b)
        .expect("graph B's configured check should exist");

    let ((status_a, body_a), (status_b, body_b)) =
        tokio::join!(get(&router_a, "/ready"), get(&router_b, "/ready"));
    assert_eq!(status_a, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body_a, r#"{"status":"DOWN"}"#);
    assert_eq!(status_b, StatusCode::OK);
    assert_eq!(body_b, r#"{"status":"UP"}"#);
}

#[tokio::test]
async fn a_healthy_graph_does_not_make_another_graph_ready() {
    let config = config(vec!["dependency".to_owned()]);
    let healthy_graph =
        router_with_health_registry(&config, state_registry("dependency", Arc::new(true)))
            .expect("healthy graph router should build");
    let unhealthy_graph =
        router_with_health_registry(&config, state_registry("dependency", Arc::new(false)))
            .expect("unhealthy graph router should build");

    let (healthy, unhealthy) = tokio::join!(
        get(&healthy_graph, "/ready"),
        get(&unhealthy_graph, "/ready")
    );
    assert_eq!(healthy.0, StatusCode::OK);
    assert_eq!(unhealthy.0, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn liveness_does_not_execute_dependency_checks() {
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_check = Arc::clone(&calls);
    let mut registry = HealthRegistry::new();
    registry
        .register("dependency", move || {
            let calls = Arc::clone(&calls_for_check);
            async move {
                calls.fetch_add(1, Ordering::Relaxed);
                false
            }
        })
        .unwrap();
    let router = router_with_health_registry(&config(vec![]), registry).unwrap();

    for path in ["/live", "/health/live", "/actuator/health/liveness"] {
        let (status, body) = get(&router, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(body, r#"{"status":"UP"}"#, "{path}");
    }
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn readiness_and_aggregate_health_execute_only_selected_checks() {
    let selected_calls = Arc::new(AtomicUsize::new(0));
    let unselected_calls = Arc::new(AtomicUsize::new(0));
    let mut registry = HealthRegistry::new();
    let selected = Arc::clone(&selected_calls);
    registry
        .register("selected", move || {
            let selected = Arc::clone(&selected);
            async move {
                selected.fetch_add(1, Ordering::Relaxed);
                true
            }
        })
        .unwrap();
    let unselected = Arc::clone(&unselected_calls);
    registry
        .register("unselected", move || {
            let unselected = Arc::clone(&unselected);
            async move {
                unselected.fetch_add(1, Ordering::Relaxed);
                false
            }
        })
        .unwrap();
    let router = router_with_health_registry(&config(vec!["selected".to_owned()]), registry)
        .expect("the selected check should be registered");

    assert_eq!(get(&router, "/ready").await.0, StatusCode::OK);
    assert_eq!(get(&router, "/actuator/health").await.0, StatusCode::OK);
    assert_eq!(selected_calls.load(Ordering::Relaxed), 2);
    assert_eq!(unselected_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn empty_selection_selects_every_check_and_an_empty_registry_is_healthy() {
    let failing = state_registry("unavailable", Arc::new(false));
    let router = router_with_health_registry(&config(vec![]), failing).unwrap();
    assert_eq!(
        get(&router, "/ready").await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );

    let empty_router = router_with_health_registry(&config(vec![]), HealthRegistry::new()).unwrap();
    assert_eq!(get(&empty_router, "/ready").await.0, StatusCode::OK);
}

#[tokio::test]
async fn wildcard_selection_runs_all_checks() {
    let router = router_with_health_registry(
        &config(vec!["*".to_owned()]),
        state_registry("unavailable", Arc::new(false)),
    )
    .expect("wildcard selection should accept every registry check");
    assert_eq!(
        get(&router, "/ready").await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn registry_supports_direct_async_health_evaluation() {
    let registry = state_registry("application", Arc::new(true));
    assert!(registry.is_healthy(&HealthConfig::default()).await);
}

#[test]
fn unknown_configured_names_are_rejected_without_echoing_configuration() {
    let error = router_with_health_registry(
        &config(vec!["private-config-value".to_owned()]),
        HealthRegistry::new(),
    )
    .expect_err("unknown configured check should fail router construction");
    assert!(matches!(
        error,
        HealthRegistryError::UnknownConfiguredChecks { count: 1 }
    ));
    assert!(!error.to_string().contains("private-config-value"));

    let mut registry = HealthRegistry::new();
    registry.register("known", || async { true }).unwrap();
    let wildcard_with_unknown = HealthConfig {
        checks: vec!["*".to_owned(), "private-config-value".to_owned()],
        ..HealthConfig::default()
    };
    let error = registry
        .validate(&wildcard_with_unknown)
        .expect_err("wildcards should not hide misspelled configured names");
    assert!(matches!(
        error,
        HealthRegistryError::UnknownConfiguredChecks { count: 1 }
    ));
    assert!(!error.to_string().contains("private-config-value"));
}

#[test]
fn duplicate_and_empty_registration_names_are_rejected() {
    let mut registry = HealthRegistry::new();
    registry.register("same", || async { true }).unwrap();
    assert!(matches!(
        registry.register("same", || async { false }),
        Err(HealthRegistryError::DuplicateCheckName(name)) if name == "same"
    ));

    assert!(matches!(
        registry.register("", || async { true }),
        Err(HealthRegistryError::EmptyCheckName)
    ));
    assert!(matches!(
        registry.register(" \t\n", || async { true }),
        Err(HealthRegistryError::EmptyCheckName)
    ));
}

struct DropSignal(Arc<AtomicBool>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

#[tokio::test]
async fn timed_out_check_fails_closed_and_drops_its_future() {
    let dropped = Arc::new(AtomicBool::new(false));
    let dropped_for_check = Arc::clone(&dropped);
    let mut registry = HealthRegistry::new();
    registry
        .register("slow", move || {
            let dropped = Arc::clone(&dropped_for_check);
            async move {
                let _drop_signal = DropSignal(dropped);
                tokio::time::sleep(Duration::from_secs(60)).await;
                true
            }
        })
        .unwrap();
    let config = ActuatorConfig {
        health: HealthConfig {
            checks: vec!["slow".to_owned()],
            check_timeout_ms: 10,
        },
        ..ActuatorConfig::all()
    };
    let router = router_with_health_registry(&config, registry).unwrap();
    let (status, body) = tokio::time::timeout(Duration::from_secs(2), get(&router, "/ready"))
        .await
        .expect("the health check timeout should bound request handling");
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body, r#"{"status":"DOWN"}"#);
    assert!(dropped.load(Ordering::Relaxed));
}

struct PollPanic;

impl Future for PollPanic {
    type Output = bool;

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        panic!("poll-panic-secret");
    }
}

#[tokio::test]
async fn check_creation_and_poll_panics_fail_closed_without_http_payloads() {
    let mut creation_registry = HealthRegistry::new();
    creation_registry
        .register("creation", || -> std::future::Ready<bool> {
            panic!("creation-panic-secret");
        })
        .unwrap();
    let creation_router =
        router_with_health_registry(&config(vec!["creation".to_owned()]), creation_registry)
            .unwrap();
    let (status, body) = get(&creation_router, "/ready").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(!body.contains("creation-panic-secret"));

    let mut polling_registry = HealthRegistry::new();
    polling_registry.register("polling", || PollPanic).unwrap();
    let polling_router =
        router_with_health_registry(&config(vec!["polling".to_owned()]), polling_registry).unwrap();
    let (status, body) = get(&polling_router, "/ready").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(!body.contains("poll-panic-secret"));
}

#[tokio::test]
async fn false_and_true_results_map_to_down_and_up() {
    let failing = router_with_health_registry(
        &config(vec!["check".to_owned()]),
        state_registry("check", Arc::new(false)),
    )
    .unwrap();
    let healthy = router_with_health_registry(
        &config(vec!["check".to_owned()]),
        state_registry("check", Arc::new(true)),
    )
    .unwrap();

    assert_eq!(
        get(&failing, "/ready").await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(get(&healthy, "/ready").await.0, StatusCode::OK);
}

#[tokio::test]
async fn authentication_precedes_protected_health_check_execution() {
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_check = Arc::clone(&calls);
    let mut registry = HealthRegistry::new();
    registry
        .register("dependency", move || {
            let calls = Arc::clone(&calls_for_check);
            async move {
                calls.fetch_add(1, Ordering::Relaxed);
                true
            }
        })
        .unwrap();
    let config = ActuatorConfig {
        security: ActuatorSecurity {
            enabled: true,
            bearer_token: Some("local-test-token".to_owned()),
        },
        ..config(vec!["dependency".to_owned()])
    };
    let router = router_with_health_registry(&config, registry).unwrap();

    let (status, body) = get(&router, "/ready").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(body.is_empty());
    assert_eq!(calls.load(Ordering::Relaxed), 0);

    let (status, body) = get_with_auth(&router, "/ready", Some("Bearer wrong-token")).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(!body.contains("local-test-token"));
    assert_eq!(calls.load(Ordering::Relaxed), 0);

    let (status, _) = get_with_auth(&router, "/ready", Some("Bearer local-test-token")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn actuator_aliases_and_reserved_routes_remain_unchanged() {
    let config = ActuatorConfig::all();
    let router = router_with_health_registry(&config, HealthRegistry::new()).unwrap();
    for path in ["/health", "/actuator/health", "/ready", "/health/ready"] {
        assert_eq!(get(&router, path).await.0, StatusCode::OK, "{path}");
    }
    let reserved = scafra_actuator::reserved_routes(&config);
    assert_eq!(reserved.len(), 12);
    assert!(reserved.contains(&("GET", "/actuator/health/readiness")));

    let disabled =
        router_with_health_registry(&ActuatorConfig::default(), HealthRegistry::new()).unwrap();
    assert_eq!(get(&disabled, "/ready").await.0, StatusCode::NOT_FOUND);
}
