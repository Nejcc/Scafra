use std::sync::atomic::{AtomicBool, Ordering};

#[tokio::test]
async fn invalid_actuator_security_stops_startup_before_serving_requests() {
    let _enabled = EnvironmentOverride::set("SCAFRA_ACTUATOR_SECURITY_ENABLED", "true");
    let _token = EnvironmentOverride::set("SCAFRA_ACTUATOR_SECURITY_BEARER_TOKEN", "'   '");
    // If typed validation regresses, this malformed address makes startup
    // return before binding a socket, so this regression test cannot hang.
    let _host = EnvironmentOverride::set("SCAFRA_SERVER_HOST", "not-an-ip-address");
    let router_builder_called = AtomicBool::new(false);

    let result = scafra::run_with_router(|_| {
        router_builder_called.store(true, Ordering::Release);
        Ok(axum::Router::new())
    })
    .await;

    let error = result.expect_err("invalid actuator security must stop application startup");
    let scafra::StartupError::Config(error) = error else {
        panic!("invalid actuator security should fail typed configuration validation");
    };
    assert!(error
        .validation_details()
        .is_some_and(|details| details.contains("actuator.security.bearer_token")));
    assert!(
        !router_builder_called.load(Ordering::Acquire),
        "the application router must not be built after configuration validation fails"
    );
}

struct EnvironmentOverride {
    key: &'static str,
    original: Option<std::ffi::OsString>,
}

impl EnvironmentOverride {
    fn set(key: &'static str, value: &str) -> Self {
        let original = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, original }
    }
}

impl Drop for EnvironmentOverride {
    fn drop(&mut self) {
        match self.original.take() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}
