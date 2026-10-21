use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use scafra_actuator::{
    router, try_router, ActuatorConfig, HealthCheckFuture, HealthCheckRegistration, HealthConfig,
};
use tower::ServiceExt;

const UNKNOWN_CHECK_NAME: &str = "unknown-legacy-check";

async fn legacy_check() -> bool {
    true
}

scafra_actuator::register_health_check!("legacy-compatibility", legacy_check);

#[tokio::test]
async fn legacy_macro_still_registers_for_compatibility_router() {
    let config = ActuatorConfig {
        health: HealthConfig {
            checks: vec!["legacy-compatibility".to_owned()],
            ..HealthConfig::default()
        },
        ..ActuatorConfig::all()
    };
    let response = router(&config)
        .oneshot(
            Request::builder()
                .uri("/ready")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("router should respond");
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024)
        .await
        .expect("response body should be readable");
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_ref(), br#"{"status":"UP"}"#);
}

#[tokio::test]
async fn legacy_router_fails_closed_for_unknown_configured_check_names() {
    let config = ActuatorConfig {
        health: HealthConfig {
            checks: vec![
                "legacy-compatibility".to_owned(),
                UNKNOWN_CHECK_NAME.to_owned(),
            ],
            ..HealthConfig::default()
        },
        ..ActuatorConfig::all()
    };
    let router = router(&config);
    for path in ["/ready", "/health/ready", "/actuator/health/readiness"] {
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
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{path}");
        let body = to_bytes(response.into_body(), 1024)
            .await
            .expect("response body should be readable");
        assert!(!String::from_utf8_lossy(&body).contains(UNKNOWN_CHECK_NAME));
    }

    for path in ["/live", "/health/live", "/actuator/health/liveness"] {
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
        assert_eq!(response.status(), StatusCode::OK, "{path}");
    }
}

#[test]
fn fallible_legacy_router_rejects_unknown_names_before_serving() {
    let config = ActuatorConfig {
        health: HealthConfig {
            checks: vec![UNKNOWN_CHECK_NAME.to_owned()],
            ..HealthConfig::default()
        },
        ..ActuatorConfig::all()
    };
    let error = try_router(&config).expect_err("unknown names must fail router construction");
    assert!(error.to_string().contains("unknown check name"));
}

#[test]
fn legacy_public_types_and_macro_reexports_remain_available() {
    let _: Option<HealthCheckRegistration> = None;
    let _: Option<HealthCheckFuture> = None;
}
