use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use scafra_web::{build_router_with_actuator, ActuatorConfig, ActuatorSecurity, HealthConfig};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tower::util::ServiceExt;

async fn unavailable_dependency() -> bool {
    false
}

static SLOW_CHECK_DROPPED: AtomicBool = AtomicBool::new(false);

struct DropSignal;

impl Drop for DropSignal {
    fn drop(&mut self) {
        SLOW_CHECK_DROPPED.store(true, Ordering::SeqCst);
    }
}

async fn slow_dependency() -> bool {
    let _drop_signal = DropSignal;
    tokio::time::sleep(Duration::from_secs(60)).await;
    true
}

async fn panicking_dependency() -> bool {
    panic!("dependency check panicked");
}

// Keep this failing registration isolated in its own integration-test binary.
// The inventory is immutable, so parallel tests all observe the same check.
scafra_web::register_health_check!("dependency", unavailable_dependency);
scafra_web::register_health_check!("slow-dependency", slow_dependency);
scafra_web::register_health_check!("panicking-dependency", panicking_dependency);

async fn get(config: &ActuatorConfig, path: &str) -> (StatusCode, String) {
    let response = build_router_with_actuator(config)
        .expect("the actuator router should build")
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
        String::from_utf8(body.to_vec()).expect("body should be UTF-8"),
    )
}

fn config_with_dependency_check() -> ActuatorConfig {
    ActuatorConfig {
        health: HealthConfig {
            checks: vec!["dependency".to_owned()],
            ..HealthConfig::default()
        },
        ..ActuatorConfig::all()
    }
}

#[tokio::test]
async fn failed_registered_check_marks_every_readiness_alias_down() {
    let config = config_with_dependency_check();

    for path in ["/ready", "/health/ready", "/actuator/health/readiness"] {
        let (status, body) = get(&config, path).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{path}");
        assert_eq!(body, r#"{"status":"DOWN"}"#, "{path}");
    }
}

#[tokio::test]
async fn failed_registered_check_does_not_mark_liveness_aliases_down() {
    let config = config_with_dependency_check();

    for path in ["/live", "/health/live", "/actuator/health/liveness"] {
        let (status, body) = get(&config, path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(body, r#"{"status":"UP"}"#, "{path}");
    }
}

#[tokio::test]
async fn aggregate_health_keeps_evaluating_the_configured_checks() {
    let config = config_with_dependency_check();

    for path in ["/health", "/actuator/health"] {
        let (status, body) = get(&config, path).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{path}");
        assert_eq!(body, r#"{"status":"DOWN"}"#, "{path}");
    }
}

#[tokio::test]
async fn unknown_readiness_check_selection_fails_closed() {
    let config = ActuatorConfig {
        health: HealthConfig {
            checks: vec!["another-check".to_owned()],
            ..HealthConfig::default()
        },
        ..ActuatorConfig::all()
    };

    for path in [
        "/ready",
        "/health/ready",
        "/actuator/health/readiness",
        "/health",
        "/actuator/health",
    ] {
        let (status, body) = get(&config, path).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{path}");
        assert_eq!(body, r#"{"status":"DOWN"}"#, "{path}");
    }
}

#[tokio::test]
async fn timed_out_check_marks_readiness_down_and_drops_its_future() {
    SLOW_CHECK_DROPPED.store(false, Ordering::SeqCst);
    let config = ActuatorConfig {
        health: HealthConfig {
            checks: vec!["slow-dependency".to_owned()],
            check_timeout_ms: 20,
        },
        ..ActuatorConfig::all()
    };

    let (status, body) = tokio::time::timeout(Duration::from_secs(2), get(&config, "/ready"))
        .await
        .expect("the configured health-check timeout should bound the probe");
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body, r#"{"status":"DOWN"}"#);
    assert!(SLOW_CHECK_DROPPED.load(Ordering::SeqCst));
}

#[tokio::test]
async fn panicking_check_marks_readiness_down() {
    let config = ActuatorConfig {
        health: HealthConfig {
            checks: vec!["panicking-dependency".to_owned()],
            ..HealthConfig::default()
        },
        ..ActuatorConfig::all()
    };

    let (status, body) = get(&config, "/ready").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body, r#"{"status":"DOWN"}"#);
}

#[tokio::test]
async fn bearer_authentication_covers_aggregate_liveness_and_readiness_routes() {
    let config = ActuatorConfig {
        security: ActuatorSecurity {
            enabled: true,
            bearer_token: Some("probe-token".to_owned()),
        },
        ..config_with_dependency_check()
    };

    for path in [
        "/health",
        "/actuator/health",
        "/live",
        "/health/live",
        "/actuator/health/liveness",
        "/ready",
        "/health/ready",
        "/actuator/health/readiness",
    ] {
        let (status, _) = get(&config, path).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}");
    }
}
