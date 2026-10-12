use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use scafra_actuator::{router_with_health_registry, ActuatorConfig, HealthConfig, HealthRegistry};
use std::{
    future::Future,
    pin::Pin,
    process::Command,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::Duration,
};
use tower::ServiceExt;

const CREATION_PANIC_PAYLOAD: &str = "creation-panic-secret";
const POLLING_PANIC_PAYLOAD: &str = "poll-panic-secret";
const UNRELATED_PANIC_PAYLOAD: &str = "unrelated panic diagnostic";

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

#[tokio::test]
async fn readiness_and_aggregate_health_run_only_selected_checks() {
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
        .expect("selected check should register");
    let unselected = Arc::clone(&unselected_calls);
    registry
        .register("unselected", move || {
            let unselected = Arc::clone(&unselected);
            async move {
                unselected.fetch_add(1, Ordering::Relaxed);
                false
            }
        })
        .expect("unselected check should register");
    let router = router_with_health_registry(&config(vec!["selected".to_owned()]), registry)
        .expect("the selected check should be registered");

    assert_eq!(get(&router, "/ready").await.0, StatusCode::OK);
    assert_eq!(get(&router, "/actuator/health").await.0, StatusCode::OK);
    assert_eq!(selected_calls.load(Ordering::Relaxed), 2);
    assert_eq!(unselected_calls.load(Ordering::Relaxed), 0);
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
        .expect("dependency check should register");
    let router = router_with_health_registry(&config(vec![]), registry)
        .expect("router should build with all checks selected");

    for path in ["/live", "/health/live", "/actuator/health/liveness"] {
        let (status, body) = get(&router, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(body, r#"{"status":"UP"}"#, "{path}");
    }
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn healthy_and_failing_checks_map_to_up_and_down_for_health_and_readiness() {
    let failing = router_with_health_registry(
        &config(vec!["check".to_owned()]),
        state_registry("check", Arc::new(false)),
    )
    .expect("failing check router should build");
    let healthy = router_with_health_registry(
        &config(vec!["check".to_owned()]),
        state_registry("check", Arc::new(true)),
    )
    .expect("healthy check router should build");

    for path in ["/ready", "/health/ready", "/actuator/health/readiness"] {
        assert_eq!(
            get(&failing, path).await.0,
            StatusCode::SERVICE_UNAVAILABLE,
            "{path}"
        );
        assert_eq!(get(&healthy, path).await.0, StatusCode::OK, "{path}");
    }
    for path in ["/health", "/actuator/health"] {
        assert_eq!(
            get(&failing, path).await.0,
            StatusCode::SERVICE_UNAVAILABLE,
            "{path}"
        );
        assert_eq!(get(&healthy, path).await.0, StatusCode::OK, "{path}");
    }
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
        .expect("slow check should register");
    let config = ActuatorConfig {
        health: HealthConfig {
            checks: vec!["slow".to_owned()],
            check_timeout_ms: 10,
        },
        ..ActuatorConfig::all()
    };
    let router = router_with_health_registry(&config, registry).expect("router should build");
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
        panic!("{POLLING_PANIC_PAYLOAD}");
    }
}

#[tokio::test]
async fn check_creation_and_poll_panics_fail_closed_without_http_payloads() {
    let mut creation_registry = HealthRegistry::new();
    creation_registry
        .register("creation", || -> std::future::Ready<bool> {
            panic!("{CREATION_PANIC_PAYLOAD}");
        })
        .expect("creation panic check should register");
    let creation_router =
        router_with_health_registry(&config(vec!["creation".to_owned()]), creation_registry)
            .expect("router should build");
    let (status, body) = get(&creation_router, "/ready").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(!body.contains(CREATION_PANIC_PAYLOAD));

    let mut polling_registry = HealthRegistry::new();
    polling_registry
        .register("polling", || PollPanic)
        .expect("poll panic check should register");
    let polling_router =
        router_with_health_registry(&config(vec!["polling".to_owned()]), polling_registry)
            .expect("router should build");
    let (status, body) = get(&polling_router, "/ready").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(!body.contains(POLLING_PANIC_PAYLOAD));

    let unrelated_panic = std::panic::catch_unwind(|| panic!("{UNRELATED_PANIC_PAYLOAD}"));
    assert!(unrelated_panic.is_err(), "unrelated panic should be caught");
}

#[test]
fn caught_health_check_panics_do_not_write_payloads_to_stderr() {
    let output = Command::new(std::env::current_exe().expect("test executable path should exist"))
        .args([
            "--exact",
            "check_creation_and_poll_panics_fail_closed_without_http_payloads",
            "--nocapture",
        ])
        .output()
        .expect("panic regression test should run in a child process");
    assert!(output.status.success(), "child panic test should pass");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains(CREATION_PANIC_PAYLOAD),
        "health-check creation panic payload was written to stderr"
    );
    assert!(
        !stderr.contains(POLLING_PANIC_PAYLOAD),
        "health-check polling panic payload was written to stderr"
    );
    assert!(
        stderr.contains(UNRELATED_PANIC_PAYLOAD),
        "the previously installed hook should handle unrelated panics"
    );
}
