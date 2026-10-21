use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use scafra_actuator::{
    router_with_health_registry, ActuatorConfig, ActuatorSecurity, HealthConfig, HealthRegistry,
    HealthRegistryError,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tower::ServiceExt;

const PRIVATE_CHECK_NAME: &str = "private-config-value";
const PRIVATE_BEARER_TOKEN: &str = "do-not-show-this-bearer-token";

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
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(path)
                .body(Body::empty())
                .expect("request should build"),
        )
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

async fn query_concurrently(router: &Router, expected: StatusCode) {
    let mut requests = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let router = router.clone();
        requests.spawn(async move { get(&router, "/ready").await.0 });
    }
    while let Some(result) = requests.join_next().await {
        assert_eq!(result.expect("request task should finish"), expected);
    }
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
async fn concurrent_requests_keep_using_their_own_router_registry() {
    let calls_a = Arc::new(AtomicUsize::new(0));
    let calls_b = Arc::new(AtomicUsize::new(0));
    let mut registry_a = HealthRegistry::new();
    let check_calls_a = Arc::clone(&calls_a);
    registry_a
        .register("dependency", move || {
            let calls = Arc::clone(&check_calls_a);
            async move {
                calls.fetch_add(1, Ordering::Relaxed);
                false
            }
        })
        .expect("graph A should register its local check");
    let mut registry_b = HealthRegistry::new();
    let check_calls_b = Arc::clone(&calls_b);
    registry_b
        .register("dependency", move || {
            let calls = Arc::clone(&check_calls_b);
            async move {
                calls.fetch_add(1, Ordering::Relaxed);
                true
            }
        })
        .expect("graph B should register its local check");
    let config = config(vec!["dependency".to_owned()]);
    let router_a =
        router_with_health_registry(&config, registry_a).expect("graph A router should build");
    let router_b =
        router_with_health_registry(&config, registry_b).expect("graph B router should build");

    let ((), ()) = tokio::join!(
        query_concurrently(&router_a, StatusCode::SERVICE_UNAVAILABLE),
        query_concurrently(&router_b, StatusCode::OK),
    );
    assert_eq!(calls_a.load(Ordering::Relaxed), 8);
    assert_eq!(calls_b.load(Ordering::Relaxed), 8);
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

#[tokio::test]
async fn repeated_configured_names_run_the_registered_check_once_per_evaluation() {
    let calls = Arc::new(AtomicUsize::new(0));
    let check_calls = Arc::clone(&calls);
    let mut registry = HealthRegistry::new();
    registry
        .register("database", move || {
            let calls = Arc::clone(&check_calls);
            async move {
                calls.fetch_add(1, Ordering::Relaxed);
                true
            }
        })
        .expect("the database check should register");
    let config = config(vec!["database".to_owned(), "database".to_owned()]);
    let router = router_with_health_registry(&config, registry)
        .expect("repeated selection names should be accepted");

    assert_eq!(get(&router, "/ready").await.0, StatusCode::OK);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn unknown_configured_names_are_identified_without_exposing_credentials() {
    let mut actuator = config(vec![PRIVATE_CHECK_NAME.to_owned()]);
    actuator.security = ActuatorSecurity {
        enabled: true,
        bearer_token: Some(PRIVATE_BEARER_TOKEN.to_owned()),
    };
    let error = router_with_health_registry(&actuator, HealthRegistry::new())
        .expect_err("unknown configured check should fail router construction");
    assert!(matches!(
        &error,
        HealthRegistryError::UnknownConfiguredChecks { count: 1, names }
            if names == &[PRIVATE_CHECK_NAME.to_owned()]
    ));
    assert!(error.to_string().contains(PRIVATE_CHECK_NAME));
    assert!(!error.to_string().contains(PRIVATE_BEARER_TOKEN));

    let mut registry = HealthRegistry::new();
    registry.register("known", || async { true }).unwrap();
    let wildcard_with_unknown = HealthConfig {
        checks: vec!["*".to_owned(), PRIVATE_CHECK_NAME.to_owned()],
        ..HealthConfig::default()
    };
    let error = registry
        .validate(&wildcard_with_unknown)
        .expect_err("wildcards should not hide misspelled configured names");
    assert!(matches!(
        &error,
        HealthRegistryError::UnknownConfiguredChecks { count: 1, names }
            if names == &[PRIVATE_CHECK_NAME.to_owned()]
    ));
    assert!(error.to_string().contains(PRIVATE_CHECK_NAME));
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
