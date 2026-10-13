use std::ffi::OsString;

use scafra_config::{Config, ConfigLoader, ScafraConfig, ValidationError};
use scafra_security::{BasicAuthConfig, JwtConfig};

const ACTUATOR_TOKEN_FIELD: &str = "actuator.security.bearer_token";
const APPLICATION_TOKEN_FIELD: &str = "security.bearer_token";
const BASIC_PASSWORD_SENTINEL: &str = "issue92-basic-password-sentinel";

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
fn application_security_requires_an_authenticator_when_bearer_is_missing() {
    let mut missing_bearer = ScafraConfig::default();
    missing_bearer.security.enabled = true;
    let error = missing_bearer
        .validate()
        .expect_err("security without any authenticator must fail validation");
    assert_eq!(error.field, "security");

    let mut basic_alternative = missing_bearer.clone();
    basic_alternative.security.basic = BasicAuthConfig {
        username: Some("basic-user".to_owned()),
        password: Some(BASIC_PASSWORD_SENTINEL.to_owned()),
    };
    basic_alternative
        .validate()
        .expect("complete Basic credentials remain a valid alternative");
}

#[test]
fn invalid_jwt_provider_configuration_reports_a_safe_category() {
    let sentinel = "provider-url-secret-sentinel";
    let mut config = ScafraConfig::default();
    config.security.enabled = true;
    config.security.jwt = JwtConfig {
        enabled: true,
        jwk_set_uri: Some(format!("file:///keys?token={sentinel}")),
        ..JwtConfig::default()
    };

    let error = config
        .validate()
        .expect_err("unsupported provider schemes must fail startup validation");
    assert_eq!(error.field, "security.jwt.jwk_set_uri");
    assert!(error.message.contains("HTTP(S) URL"));
    assert!(!format!("{error} {error:?}").contains(sentinel));
}

#[test]
fn application_security_rejects_blank_bearer_even_with_complete_basic_auth() {
    for token in ["", " ", "   ", "\t", " \t "] {
        let mut config = ScafraConfig::default();
        config.security.enabled = true;
        config.security.bearer_token = Some(token.to_owned());
        config.security.basic = BasicAuthConfig {
            username: Some("basic-user".to_owned()),
            password: Some(BASIC_PASSWORD_SENTINEL.to_owned()),
        };

        let error = config
            .validate()
            .expect_err("a configured blank bearer token must fail typed validation");
        assert_eq!(error.field, APPLICATION_TOKEN_FIELD);
        assert!(error.message.contains("empty or whitespace-only"));
        let diagnostics = format!("{error} {error:?}");
        assert!(!diagnostics.contains(BASIC_PASSWORD_SENTINEL));
    }
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

#[test]
fn environment_application_bearer_tokens_use_the_same_blank_validation_rules() {
    let prefix = format!("SCFRA_ISSUE92_{}", std::process::id());
    let enabled_key = format!("{prefix}_SECURITY__ENABLED");
    let token_key = format!("{prefix}_SECURITY__BEARER_TOKEN");
    let username_key = format!("{prefix}_SECURITY__BASIC__USERNAME");
    let password_key = format!("{prefix}_SECURITY__BASIC__PASSWORD");
    let port_key = format!("{prefix}_SERVER__PORT");
    let _enabled_restore = RestoreEnvironment::new(&enabled_key);
    let _token_restore = RestoreEnvironment::new(&token_key);
    let _username_restore = RestoreEnvironment::new(&username_key);
    let _password_restore = RestoreEnvironment::new(&password_key);
    let _port_restore = RestoreEnvironment::new(&port_key);
    std::env::set_var(&enabled_key, "true");
    std::env::set_var(&username_key, "basic-user");
    std::env::set_var(&password_key, BASIC_PASSWORD_SENTINEL);

    let loader = ConfigLoader::new().env_prefix(&prefix);
    for token in ["", " ", "   ", "\t", " \t "] {
        // Environment values use the existing YAML scalar parser, so quote
        // whitespace strings to preserve them as strings through the override.
        let encoded = serde_yaml::to_string(token)
            .expect("whitespace token should serialize as a YAML scalar");
        std::env::set_var(&token_key, encoded);
        let error = loader
            .load_validated::<ScafraConfig>()
            .expect_err("blank environment bearer tokens must fail before startup");
        assert_eq!(
            error.validation_details(),
            Some("security.bearer_token: must not be empty or whitespace-only")
        );
        let diagnostics = format!("{error} {error:?} {:?}", error.validation_details());
        assert!(!diagnostics.contains(BASIC_PASSWORD_SENTINEL));
    }

    std::env::set_var(&token_key, "environment-token-sentinel");
    let config = loader
        .load_validated::<ScafraConfig>()
        .expect("a valid environment bearer token should pass typed validation");
    assert_eq!(
        config.security.bearer_token.as_deref(),
        Some("environment-token-sentinel")
    );

    std::env::set_var(&port_key, "0");
    let error = loader
        .load_validated::<ScafraConfig>()
        .expect_err("invalid startup settings should fail typed validation");
    let diagnostics = format!("{error} {error:?} {:?}", error.validation_details());
    assert!(error
        .validation_details()
        .is_some_and(|details| details.contains("server.port")));
    assert!(!diagnostics.contains("environment-token-sentinel"));
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
