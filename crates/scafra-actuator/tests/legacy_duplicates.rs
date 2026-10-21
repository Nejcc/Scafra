use axum::{body::Body, http::Request, http::StatusCode};
use scafra_actuator::{router, try_router, ActuatorConfig, HealthRegistryError};
use tower::ServiceExt;

async fn first() -> bool {
    true
}

async fn second() -> bool {
    true
}

scafra_actuator::register_health_check!("duplicate", first);
scafra_actuator::register_health_check!("duplicate", second);

#[test]
fn fallible_legacy_startup_rejects_duplicate_registration_names() {
    let error = try_router(&ActuatorConfig::all())
        .expect_err("duplicate process-wide names must be rejected at startup");
    assert!(matches!(
        error,
        HealthRegistryError::DuplicateCheckName(name) if name == "duplicate"
    ));
}

#[tokio::test]
async fn compatibility_router_fails_closed_for_duplicate_registration_names() {
    let response = router(&ActuatorConfig::all())
        .oneshot(
            Request::builder()
                .uri("/ready")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("router should respond");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}
