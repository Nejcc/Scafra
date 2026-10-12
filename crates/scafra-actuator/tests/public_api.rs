use axum::{body::Body, http::Request, http::StatusCode};
use scafra_actuator::{
    reserved_routes, router, router_with_health_registry, ActuatorConfig, HealthCheckFuture,
    HealthCheckRegistration, HealthRegistry,
};
use tower::ServiceExt;

#[tokio::test]
async fn public_root_imports_build_both_compatibility_and_graph_scoped_routers() {
    let config = ActuatorConfig::all();
    let legacy_router = router(&config);
    let graph_router = router_with_health_registry(&config, HealthRegistry::new())
        .expect("the public graph-scoped constructor should build");
    assert_eq!(reserved_routes(&config).len(), 12);

    let response = graph_router
        .oneshot(
            Request::builder()
                .uri("/ready")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("router should respond");
    assert_eq!(response.status(), StatusCode::OK);
    drop(legacy_router);

    let _: Option<HealthCheckRegistration> = None;
    let _: Option<HealthCheckFuture> = None;
}

#[test]
fn exported_legacy_registration_macro_remains_available_at_the_crate_root() {
    async fn process_check() -> bool {
        true
    }

    scafra_actuator::register_health_check!("public-api-check", process_check);
}
