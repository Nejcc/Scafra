//! Experimental: compose a Scafra controller and a RustClamp route, served by
//! Scafra's standard runner.

use axum::{Router, routing::get};
use rustclamp_core::{ContributionTarget, ModuleId};
use rustclamp_http::{HttpRoute, HttpRoutes, Public, RouteBuildError};
use scafra::{ScafraConfig, StartupError, web::WebError};

#[scafra::service]
struct GreetingService;

impl GreetingService {
    fn greet(&self, name: &str) -> String {
        format!("Hello, {name}!")
    }
}

#[scafra::controller("/api")]
#[derive(Default)]
struct GreetingController {
    service: GreetingService,
}

// Standalone `build_router()` constructs only `Default` controllers.
#[scafra::routes(default)]
impl GreetingController {
    #[get("/hello/{name}")]
    async fn hello(&self, name: scafra::prelude::Path<String>) -> String {
        self.service.greet(&name.0)
    }
}

fn compose(scafra_router: Router) -> Result<Router, RouteBuildError> {
    HttpRoutes::<Public>::new().build(&[
        (
            ModuleId::new("app.scafra"),
            HttpRoute::<Public>::mount("/scafra", scafra_router),
        ),
        (
            ModuleId::new("app.health"),
            HttpRoute::<Public>::new("/health", get(|| async { "ok" })),
        ),
    ])
}

/// Builds the composed router once Scafra has loaded validated configuration.
fn build(config: &ScafraConfig) -> Result<Router, StartupError> {
    let scafra_router =
        scafra::web::build_router_with_actuator_and_security(&config.actuator, &config.security)?;
    compose(scafra_router).map_err(route_error)
}

// ponytail: StartupError has no variant for external errors; reported as a
// web startup failure until Scafra offers one.
fn route_error(error: RouteBuildError) -> StartupError {
    WebError::Server(std::io::Error::new(std::io::ErrorKind::InvalidInput, error)).into()
}

#[tokio::main]
async fn main() {
    // Scafra owns configuration, lifecycle, scheduler, signals and shutdown.
    if let Err(error) = scafra::run_with_router(build).await {
        // Lifecycle errors may contain application-provided sensitive text.
        eprintln!("Scafra startup failed ({})", error.kind());
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn serves_generated_scafra_controller_and_clamp_route() {
        let router = build(&ScafraConfig::default()).unwrap();
        for (path, expected) in [
            ("/scafra/api/hello/Alice", "Hello, Alice!"),
            ("/health", "ok"),
        ] {
            let response = router
                .clone()
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = to_bytes(response.into_body(), 1024).await.unwrap();
            assert_eq!(&body[..], expected.as_bytes());
        }
    }

    #[tokio::test]
    async fn mounting_preserves_scafra_security() {
        let security = scafra::SecurityConfig {
            enabled: true,
            hide_unauthorized: false,
            bearer_token: Some("demo-token".into()),
            ..Default::default()
        };
        let router = compose(
            scafra::web::build_router_with_actuator_and_security(
                &scafra::ActuatorConfig::default(),
                &security,
            )
            .unwrap(),
        )
        .unwrap();
        let denied = router
            .clone()
            .oneshot(
                Request::get("/scafra/api/hello/Alice")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        let allowed = router
            .clone()
            .oneshot(
                Request::get("/scafra/api/hello/Alice")
                    .header("authorization", "Bearer demo-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(allowed.status(), StatusCode::OK);
        // The explicitly public Clamp route is outside Scafra's security layer.
        let health = router
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(health.status(), StatusCode::OK);
    }

    #[test]
    fn rejects_a_contribution_inside_the_scafra_mount() {
        let result = HttpRoutes::<Public>::new().build(&[
            (
                ModuleId::new("app.scafra"),
                HttpRoute::<Public>::mount("/scafra", Router::new()),
            ),
            (
                ModuleId::new("app.conflict"),
                HttpRoute::<Public>::new("/scafra/other", get(|| async { "conflict" })),
            ),
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn composition_failure_is_a_startup_error() {
        let conflict = HttpRoutes::<Public>::new()
            .build(&[
                (
                    ModuleId::new("app.scafra"),
                    HttpRoute::<Public>::mount("/scafra", Router::new()),
                ),
                (
                    ModuleId::new("app.conflict"),
                    HttpRoute::<Public>::new("/scafra/other", get(|| async { "conflict" })),
                ),
            ])
            .unwrap_err();
        assert_eq!(route_error(conflict).kind(), "web");
    }

    #[test]
    fn invalid_configuration_fails_before_serving() {
        let mut config = ScafraConfig::default();
        config.actuator.health.checks = vec!["missing-check".to_owned()];
        assert_eq!(build(&config).unwrap_err().kind(), "web");
    }
}
