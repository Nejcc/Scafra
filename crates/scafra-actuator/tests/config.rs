use scafra_actuator::{ActuatorConfig, ActuatorSecurity, EndpointSelection, HealthConfig};

const DESERIALIZED_TOKEN: &str = "deserialized-secret";

#[test]
fn defaults_disable_endpoints_and_use_the_documented_health_timeout() {
    let config = ActuatorConfig::default();
    assert!(config.enabled_endpoints().is_empty());
    assert_eq!(config.health.check_timeout_ms, 1000);
    assert!(config.health.checks.is_empty());
    assert!(!config.security.enabled);
    assert!(config.security.bearer_token.is_none());

    let all = ActuatorConfig::all();
    assert_eq!(
        all.enabled_endpoints(),
        ["health", "live", "ready", "info", "metrics"]
    );
}

#[test]
fn list_and_comma_separated_endpoint_selections_match_names() {
    let list = ActuatorConfig {
        endpoints: EndpointSelection::Names(vec!["metrics".to_owned(), "live".to_owned()]),
        ..ActuatorConfig::default()
    };
    assert_eq!(list.enabled_endpoints(), ["live", "metrics"]);
    assert!(list.is_enabled("live"));
    assert!(!list.is_enabled("health"));

    let pattern = ActuatorConfig {
        endpoints: EndpointSelection::Pattern("info, ready,metrics".to_owned()),
        ..ActuatorConfig::default()
    };
    assert_eq!(pattern.enabled_endpoints(), ["ready", "info", "metrics"]);
}

#[test]
fn wildcard_selection_enables_every_group_in_stable_order() {
    let config = ActuatorConfig {
        endpoints: EndpointSelection::Pattern("health,*".to_owned()),
        ..ActuatorConfig::default()
    };
    assert_eq!(
        config.enabled_endpoints(),
        ["health", "live", "ready", "info", "metrics"]
    );
}

#[test]
fn serde_preserves_config_shape_and_applies_defaults() {
    let config: ActuatorConfig =
        serde_json::from_str(r#"{"endpoints":["ready","info"],"health":{"checks":["db"]}}"#)
            .expect("configuration should deserialize");
    assert_eq!(config.enabled_endpoints(), ["ready", "info"]);
    assert_eq!(config.health.checks, ["db"]);
    assert_eq!(config.health.check_timeout_ms, 1000);
    assert!(!config.security.enabled);

    let encoded = serde_json::to_value(config).expect("configuration should serialize");
    assert_eq!(encoded["endpoints"], serde_json::json!(["ready", "info"]));
    assert_eq!(encoded["health"]["check_timeout_ms"], 1000);
    assert_eq!(encoded["security"]["enabled"], false);
}

#[test]
fn security_token_is_omitted_from_serde_and_debug_output() {
    let secret = "actuator-secret-for-redaction-test";
    let config = ActuatorConfig {
        security: ActuatorSecurity {
            enabled: true,
            bearer_token: Some(secret.to_owned()),
        },
        ..ActuatorConfig::default()
    };

    let encoded = serde_json::to_string(&config).expect("configuration should serialize");
    let debug = format!("{config:?}");
    assert!(
        !encoded.contains(secret),
        "serialized config exposed a token"
    );
    assert!(!debug.contains(secret), "debug output exposed a token");
    assert!(debug.contains("bearer_token_configured: true"));

    let encoded =
        format!(r#"{{"security":{{"enabled":true,"bearer_token":"{DESERIALIZED_TOKEN}"}}}}"#);
    let decoded: ActuatorConfig =
        serde_json::from_str(&encoded).expect("security configuration should deserialize");
    assert!(
        decoded
            .security
            .bearer_token
            .as_deref()
            .is_some_and(|value| value == DESERIALIZED_TOKEN),
        "the configured bearer token should deserialize"
    );
}

#[test]
fn health_config_default_is_consistent_when_used_directly() {
    assert_eq!(HealthConfig::default().check_timeout_ms, 1000);
}
