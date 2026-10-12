use std::ffi::OsString;

use scafra_config::{Config, ConfigLoader, ScafraConfig, SecurityConfig, ValidationError};

const ACTUATOR_TOKEN_FIELD: &str = "actuator.security.bearer_token";

fn actuator_security_config(token: Option<&str>) -> ScafraConfig {
    let mut config = ScafraConfig::default();
    config.actuator.security.enabled = true;
    config.actuator.security.bearer_token = token.map(str::to_owned);
    config
}

fn assert_actuator_token_error(error: ValidationError) {
    assert_eq!(error.field, ACTUATOR_TOKEN_FIELD);
    assert!(error.message.contains("non-blank"));
    assert!(error.message.contains("actuator security is enabled"));
}

#[test]
fn missing_actuator_bearer_token_fails_validation() {
    let error = actuator_security_config(None)
        .validate()
        .expect_err("enabled actuator security requires a bearer token");

    assert_actuator_token_error(error);
}

#[test]
fn empty_actuator_bearer_token_fails_validation() {
    let error = actuator_security_config(Some(""))
        .validate()
        .expect_err("an empty actuator bearer token must be rejected");

    assert_actuator_token_error(error);
}

#[test]
fn whitespace_only_actuator_bearer_token_fails_validation() {
    let token = " \t\n ";
    let error = actuator_security_config(Some(token))
        .validate()
        .expect_err("a whitespace-only actuator bearer token must be rejected");
    assert_actuator_token_error(error.clone());

    let diagnostics = format!("{error} {error:?}");
    assert!(!diagnostics.contains(token));
}

#[test]
fn non_blank_actuator_bearer_token_passes_validation() {
    actuator_security_config(Some("actuator-secret-sentinel"))
        .validate()
        .expect("a non-blank actuator bearer token should validate");
}

#[test]
fn application_wide_security_validation_keeps_its_existing_behavior() {
    let whitespace_application_token = ScafraConfig {
        security: SecurityConfig {
            enabled: true,
            bearer_token: Some(" \t ".to_owned()),
            ..SecurityConfig::default()
        },
        ..ScafraConfig::default()
    };
    whitespace_application_token
        .validate()
        .expect("application-wide whitespace-token behavior must remain unchanged");

    let empty_application_token = ScafraConfig {
        security: SecurityConfig {
            enabled: true,
            bearer_token: Some(String::new()),
            ..SecurityConfig::default()
        },
        ..ScafraConfig::default()
    };
    let error = empty_application_token
        .validate()
        .expect_err("application-wide empty-token behavior must remain unchanged");
    assert_eq!(error.field, "security");
}

#[test]
fn environment_actuator_tokens_use_the_same_validation_rules() {
    let prefix = format!("SCFRA_ISSUE77_{}", std::process::id());
    let enabled_key = format!("{prefix}_ACTUATOR_SECURITY_ENABLED");
    let token_key = format!("{prefix}_ACTUATOR_SECURITY_BEARER_TOKEN");
    let _enabled_restore = RestoreEnvironment::new(&enabled_key);
    let _token_restore = RestoreEnvironment::new(&token_key);
    std::env::set_var(&enabled_key, "true");

    let loader = ConfigLoader::new().env_prefix(&prefix);
    for raw_token in ["", "\"\"", "'   '"] {
        std::env::set_var(&token_key, raw_token);
        let error = loader
            .load_validated::<ScafraConfig>()
            .expect_err("empty and whitespace environment tokens must fail validation");
        assert_eq!(
            error.validation_details(),
            Some("actuator.security.bearer_token: must be a non-blank bearer token when actuator security is enabled")
        );
    }

    std::env::set_var(&token_key, "environment-token");
    let config = loader
        .load_validated::<ScafraConfig>()
        .expect("a non-blank environment token should validate");
    assert_eq!(
        config.actuator.security.bearer_token.as_deref(),
        Some("environment-token")
    );
}

struct RestoreEnvironment {
    key: String,
    original: Option<OsString>,
}

impl RestoreEnvironment {
    fn new(key: &str) -> Self {
        Self {
            key: key.to_owned(),
            original: std::env::var_os(key),
        }
    }
}

impl Drop for RestoreEnvironment {
    fn drop(&mut self) {
        match self.original.take() {
            Some(value) => std::env::set_var(&self.key, value),
            None => std::env::remove_var(&self.key),
        }
    }
}
