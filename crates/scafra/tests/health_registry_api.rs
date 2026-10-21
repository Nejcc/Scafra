use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use scafra::{ActuatorConfig, HealthRegistry};
use std::sync::Arc;
use tower::util::ServiceExt;

async fn legacy_facade_check() -> bool {
    true
}

scafra::register_health_check!("facade-legacy", legacy_facade_check);

#[tokio::test]
async fn graph_scoped_router_api_is_reexported_through_the_framework_facade() {
    let application_value = Arc::new(true);
    let value_for_check = Arc::clone(&application_value);
    let mut registry = HealthRegistry::new();
    registry
        .register("application-state", move || {
            let value = Arc::clone(&value_for_check);
            async move { *value }
        })
        .expect("application check should register");
    let config = ActuatorConfig {
        health: scafra::HealthConfig {
            checks: vec!["application-state".to_owned()],
            ..scafra::HealthConfig::default()
        },
        ..ActuatorConfig::all()
    };
    let router = scafra::web::build_router_with_actuator_and_health_registry(&config, registry)
        .expect("the public web router API should accept the registry");
    let response = router
        .oneshot(
            Request::builder()
                .uri("/ready")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("router should respond");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 1024)
        .await
        .expect("response body should be readable");
    assert_eq!(body.as_ref(), br#"{"status":"UP"}"#);
}

#[test]
fn legacy_public_api_paths_remain_available() {
    let _: Option<scafra::actuator::HealthCheckRegistration> = None;
    let _: Option<scafra::actuator::HealthCheckFuture> = None;
    let _: Option<scafra::HealthRegistryError> = None;
    let _: Option<scafra::web::HealthRegistry> = None;
}

#[test]
fn web_router_rejects_unknown_graph_scoped_health_configuration_at_construction() {
    let config = ActuatorConfig {
        health: scafra::HealthConfig {
            checks: vec!["unknown-private-value".to_owned()],
            ..scafra::HealthConfig::default()
        },
        security: scafra::ActuatorSecurity {
            enabled: true,
            bearer_token: Some("do-not-leak-this-token".to_owned()),
        },
        ..ActuatorConfig::all()
    };
    let error =
        scafra::web::build_router_with_actuator_and_health_registry(&config, HealthRegistry::new())
            .expect_err("unknown graph-scoped check should prevent router construction");
    let message = error.to_string();
    assert!(message.contains("unknown check name"));
    assert!(message.contains("unknown-private-value"));
    assert!(!message.contains("do-not-leak-this-token"));
}
