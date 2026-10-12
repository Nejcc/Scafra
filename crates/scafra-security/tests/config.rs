use scafra_security::{BasicAuthConfig, JwtConfig, SecurityConfig};

const BASIC_PASSWORD_SECRET: &str = "basic-secret-sentinel";
const JWT_SECRET: &str = "jwt-secret-sentinel";

#[test]
fn defaults_and_partial_configuration_deserialize_consistently() {
    let config = SecurityConfig::default();
    assert!(!config.enabled);
    assert!(!config.hide_unauthorized);
    assert!(config.bearer_token.is_none());
    assert!(config.basic.username.is_none());
    assert!(config.basic.password.is_none());
    assert!(config.permit_all.is_empty());
    assert!(!config.jwt.enabled);
    assert_eq!(config.validate(), Ok(()));

    let config: SecurityConfig =
        serde_json::from_str(r#"{"enabled":true,"bearer_token":"configured-token"}"#)
            .expect("the partial security configuration should deserialize");
    assert!(config.enabled);
    assert_eq!(config.bearer_token.as_deref(), Some("configured-token"));
    assert_eq!(config.validate(), Ok(()));
}

#[test]
fn invalid_authentication_configurations_return_safe_validation_errors() {
    let cases = [
        (
            SecurityConfig {
                enabled: true,
                ..SecurityConfig::default()
            },
            "a bearer token or complete basic credentials are required",
        ),
        (
            SecurityConfig {
                enabled: true,
                bearer_token: Some(String::new()),
                ..SecurityConfig::default()
            },
            "the bearer token must not be empty",
        ),
        (
            SecurityConfig {
                enabled: true,
                basic: BasicAuthConfig {
                    username: Some("user".to_owned()),
                    password: None,
                },
                ..SecurityConfig::default()
            },
            "a bearer token or complete basic credentials are required",
        ),
        (
            SecurityConfig {
                enabled: true,
                basic: BasicAuthConfig {
                    username: Some(String::new()),
                    password: Some("password".to_owned()),
                },
                ..SecurityConfig::default()
            },
            "basic credentials must not be empty",
        ),
        (
            SecurityConfig {
                enabled: true,
                basic: BasicAuthConfig {
                    username: Some("user".to_owned()),
                    password: Some(String::new()),
                },
                ..SecurityConfig::default()
            },
            "basic credentials must not be empty",
        ),
        (
            SecurityConfig {
                jwt: JwtConfig {
                    enabled: true,
                    ..JwtConfig::default()
                },
                ..SecurityConfig::default()
            },
            "JWT requires secret, issuer_uri, or jwk_set_uri",
        ),
        (
            SecurityConfig {
                jwt: JwtConfig {
                    enabled: true,
                    secret: Some(String::new()),
                    ..JwtConfig::default()
                },
                ..SecurityConfig::default()
            },
            "the JWT secret must not be empty",
        ),
    ];

    for (config, expected) in cases {
        assert_eq!(config.validate(), Err(expected));
    }
}

#[test]
fn application_bearer_validation_keeps_its_exact_empty_string_behavior() {
    let whitespace_token = SecurityConfig {
        enabled: true,
        bearer_token: Some(" \t ".to_owned()),
        ..SecurityConfig::default()
    };
    assert_eq!(whitespace_token.validate(), Ok(()));
}

#[test]
fn credential_config_debug_output_redacts_all_credential_values() {
    let config = SecurityConfig {
        enabled: true,
        bearer_token: Some("bearer-sentinel-secret".to_owned()),
        basic: BasicAuthConfig {
            username: Some("basic-user-sentinel".to_owned()),
            password: Some("basic-password-sentinel".to_owned()),
        },
        jwt: JwtConfig {
            enabled: true,
            secret: Some("jwt-secret-sentinel".to_owned()),
            issuer_uri: Some("https://issuer.test/?client_secret=issuer-uri-sentinel".to_owned()),
            jwk_set_uri: Some("https://keys.test/?token=jwks-uri-sentinel".to_owned()),
            ..JwtConfig::default()
        },
        ..SecurityConfig::default()
    };

    let diagnostic = format!("{config:?}");
    for sentinel in [
        "bearer-sentinel-secret",
        "basic-user-sentinel",
        "basic-password-sentinel",
        "jwt-secret-sentinel",
        "issuer-uri-sentinel",
        "jwks-uri-sentinel",
    ] {
        assert!(!diagnostic.contains(sentinel));
    }
    assert!(diagnostic.contains("[REDACTED]"));
}

#[test]
fn config_serialization_continues_to_omit_basic_and_jwt_secrets() {
    let config = SecurityConfig {
        basic: BasicAuthConfig {
            username: Some("user".to_owned()),
            password: Some(BASIC_PASSWORD_SECRET.to_owned()),
        },
        jwt: JwtConfig {
            secret: Some(JWT_SECRET.to_owned()),
            ..JwtConfig::default()
        },
        ..SecurityConfig::default()
    };

    let serialized = serde_json::to_string(&config).expect("config should serialize");
    assert!(!serialized.contains(BASIC_PASSWORD_SECRET));
    assert!(!serialized.contains(JWT_SECRET));
}
