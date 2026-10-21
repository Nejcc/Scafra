use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;

static ENVIRONMENT_LOCK: Mutex<()> = Mutex::const_new(());

#[tokio::test]
async fn unknown_readiness_name_stops_application_startup_before_serving() {
    let _environment_lock = ENVIRONMENT_LOCK.lock().await;
    let _profile = EnvironmentOverride::set("SCAFRA_PROFILE", "default");

    let result = scafra::run_with_router(|config| {
        let actuator = scafra::ActuatorConfig {
            health: scafra::HealthConfig {
                checks: vec!["misspelled-database".to_owned()],
                ..scafra::HealthConfig::default()
            },
            ..config.actuator.clone()
        };
        scafra::web::build_router_with_actuator(&actuator).map_err(scafra::StartupError::from)
    })
    .await;

    let error = result.expect_err("unknown configured checks must stop application startup");
    assert_eq!(error.kind(), "web");
    assert!(error.to_string().contains("misspelled-database"));
}

#[tokio::test]
async fn invalid_security_stops_startup_before_router_construction() {
    let _environment_lock = ENVIRONMENT_LOCK.lock().await;
    const BASIC_PASSWORD_SENTINEL: &str = "issue92-startup-basic-password-sentinel";

    let _profile = EnvironmentOverride::set("SCAFRA_PROFILE", "default");
    {
        let _application_security = EnvironmentOverride::set("SCAFRA_SECURITY_ENABLED", "false");
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
    {
        let _actuator_security =
            EnvironmentOverride::set("SCAFRA_ACTUATOR_SECURITY_ENABLED", "false");
        let _enabled = EnvironmentOverride::set("SCAFRA_SECURITY_ENABLED", "true");
        // Quote and escape the tab so the existing scalar parser retains a
        // configured whitespace string rather than interpreting it as null.
        let _token = EnvironmentOverride::set("SCAFRA_SECURITY_BEARER_TOKEN", "\" \\t \"");
        let _username = EnvironmentOverride::set("SCAFRA_SECURITY_BASIC_USERNAME", "startup-user");
        let _password =
            EnvironmentOverride::set("SCAFRA_SECURITY_BASIC_PASSWORD", BASIC_PASSWORD_SENTINEL);
        let _host = EnvironmentOverride::set("SCAFRA_SERVER_HOST", "not-an-ip-address");
        let _port = EnvironmentOverride::set("SCAFRA_SERVER_PORT", "8080");
        let router_builder_called = AtomicBool::new(false);

        let result = scafra::run_with_router(|_| {
            router_builder_called.store(true, Ordering::Release);
            Ok(axum::Router::new())
        })
        .await;

        let error = result.expect_err("a blank application token must stop startup");
        let diagnostics = format!("{error} {error:?}");
        let scafra::StartupError::Config(config_error) = error else {
            panic!("blank application token should fail typed configuration validation");
        };
        let details = config_error.validation_details().unwrap_or_else(|| {
            panic!("typed validation should identify the bearer-token field: {diagnostics}")
        });
        assert!(details.contains("security.bearer_token"));
        assert!(details.contains("empty or whitespace-only"));
        assert!(!diagnostics.contains(BASIC_PASSWORD_SENTINEL));
        assert!(!details.contains(BASIC_PASSWORD_SENTINEL));
        assert!(
            !router_builder_called.load(Ordering::Acquire),
            "the router must not be built after application bearer validation fails"
        );
    }
}

#[tokio::test]
async fn readiness_names_are_validated_against_profile_and_environment_merged_config() {
    let _environment_lock = ENVIRONMENT_LOCK.lock().await;
    let root = std::env::temp_dir().join(format!(
        "scafra-readiness-config-test-{}",
        std::process::id()
    ));
    let resources = root.join("src/resources");
    std::fs::create_dir_all(&resources).expect("the temporary config root should be created");
    std::fs::write(
        resources.join("application.yaml"),
        "scafra:\n  profiles:\n    active: probe\nactuator:\n  health:\n    checks: [base-check]\n",
    )
    .expect("base configuration should be written");
    std::fs::write(
        resources.join("application-probe.yaml"),
        "actuator:\n  health:\n    checks: [profile-check]\n",
    )
    .expect("profile configuration should be written");

    let profile_config = scafra::ConfigLoader::new()
        .root(&root)
        .load_validated::<scafra::ScafraConfig>()
        .expect("the merged profile config should pass typed validation");
    assert_eq!(profile_config.actuator.health.checks, ["profile-check"]);
    assert!(scafra::actuator::try_router(&profile_config.actuator).is_err());

    let _health_timeout =
        EnvironmentOverride::set("SCAFRA_ACTUATOR_HEALTH_CHECK_TIMEOUT_MS", "250");
    let environment_config = scafra::ConfigLoader::new()
        .root(&root)
        .load_validated::<scafra::ScafraConfig>()
        .expect("environment overrides should preserve typed config validity");
    assert_eq!(environment_config.actuator.health.checks, ["profile-check"]);
    assert_eq!(environment_config.actuator.health.check_timeout_ms, 250);
    assert!(scafra::actuator::try_router(&environment_config.actuator).is_err());

    std::fs::remove_dir_all(root).expect("the temporary config root should be removed");
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
