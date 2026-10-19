//! Compose a Scafra controller and a RustClamp route on one listener.

use std::{error::Error, time::Duration};

use axum::{Router, routing::get};
use rustclamp_core::{ContributionTarget, ModuleId};
use rustclamp_http::{HttpRoute, HttpRoutes, Public, RouteBuildError};
use scafra::{Application, ApplicationContext, ConfigLoader, ScafraConfig, Scheduler};

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let loader = match std::env::var("SCAFRA_PROFILE") {
        Ok(profile) => ConfigLoader::new().profile(profile),
        Err(_) => ConfigLoader::new(),
    };
    let config = loader.load_validated::<ScafraConfig>()?;
    scafra::foundation::init_runtime_logging(&config.logging);

    // Validate composition and bind before starting lifecycle work.
    let scafra_router =
        scafra::web::build_router_with_actuator_and_security(&config.actuator, &config.security)?;
    let router = compose(scafra_router)?;
    let listener =
        tokio::net::TcpListener::bind((config.server.host.as_str(), config.server.port)).await?;
    let address = listener.local_addr()?;

    // Watch Ctrl-C concurrently; registration failures are propagated below.
    let mut signal = tokio::spawn(tokio::signal::ctrl_c());
    let mut application = Application::new(ApplicationContext::discover());
    if let Err(error) = application.start() {
        signal.abort();
        return Err(error.into());
    }
    let scheduler = Scheduler::start(&config.scheduler);
    println!("Scafra + RustClamp listening at http://{address}");

    // Selecting explicitly propagates signal-registration errors instead of
    // panicking or waiting forever. Once signalled, ask the server to drain.
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let serving = rustclamp_http::serve(
        listener,
        router,
        async {
            let _ = stopped.await;
        },
        Duration::from_secs(10),
    );
    tokio::pin!(serving);
    let (result, reason): (Result<(), Box<dyn Error>>, _) = tokio::select! {
        result = &mut serving => (result.map_err(Into::into), scafra::ShutdownReason::RuntimeFailure),
        result = &mut signal => {
            let reason = if matches!(result, Ok(Ok(()))) {
                scafra::ShutdownReason::Signal
            } else {
                scafra::ShutdownReason::RuntimeFailure
            };
            let _ = stop.send(());
            let drain = (&mut serving).await;
            let outcome = match result {
                Ok(Ok(())) => drain.map_err(Into::into),
                Ok(Err(error)) => Err(error.into()),
                Err(error) => Err(error.into()),
            };
            (outcome, reason)
        }
    };
    signal.abort();
    scheduler.stop();
    let cleanup = application.shutdown_with(reason, scafra::ShutdownPolicy::default());
    if cleanup.is_err() {
        // Lifecycle errors may contain application-provided sensitive text.
        eprintln!("Scafra lifecycle cleanup failed");
    }
    result?;
    cleanup?;
    Ok(())
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
        let router = compose(scafra::build_router().unwrap()).unwrap();
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
}
