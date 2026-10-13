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
            "the bearer token must not be empty or whitespace-only",
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
fn jwt_provider_configuration_is_validated_without_echoing_urls() {
    let cases = [
        (
            JwtConfig {
                enabled: true,
                issuer_uri: Some("   ".to_owned()),
                ..JwtConfig::default()
            },
            "JWT issuer_uri must be a valid HTTP(S) URL",
        ),
        (
            JwtConfig {
                enabled: true,
                jwk_set_uri: Some("file:///tmp/provider-uri-sentinel".to_owned()),
                ..JwtConfig::default()
            },
            "JWT jwk_set_uri must be a valid HTTP(S) URL",
        ),
        (
            JwtConfig {
                enabled: true,
                secret: Some("hmac-secret".to_owned()),
                jwk_set_uri: Some("https://keys.test/keys".to_owned()),
                ..JwtConfig::default()
            },
            "JWT secret cannot be combined with remote JWKS configuration",
        ),
        (
            JwtConfig {
                enabled: true,
                secret: Some("hmac-secret".to_owned()),
                audiences: vec![" ".to_owned()],
                ..JwtConfig::default()
            },
            "JWT audiences must not contain empty or whitespace-only values",
        ),
    ];

    for (jwt, expected) in cases {
        let config = SecurityConfig {
            jwt,
            ..SecurityConfig::default()
        };
        let error = config.validate().expect_err("invalid JWT config must fail");
        assert_eq!(error, expected);
        assert!(!format!("{error} {config:?}").contains("provider-uri-sentinel"));
    }

    let legacy_hmac_with_issuer = SecurityConfig {
        jwt: JwtConfig {
            enabled: true,
            secret: Some("hmac-secret".to_owned()),
            issuer_uri: Some("https://issuer.test".to_owned()),
            ..JwtConfig::default()
        },
        ..SecurityConfig::default()
    };
    assert_eq!(legacy_hmac_with_issuer.validate(), Ok(()));
}

#[test]
fn application_bearer_validation_rejects_blank_values_even_with_alternatives() {
    for token in ["", " ", "   ", "\t", " \t "] {
        let config = SecurityConfig {
            enabled: true,
            bearer_token: Some(token.to_owned()),
            ..SecurityConfig::default()
        };
        assert_eq!(
            config.validate(),
            Err("the bearer token must not be empty or whitespace-only"),
            "blank token {token:?} must fail validation"
        );

        let config_with_basic = SecurityConfig {
            enabled: true,
            bearer_token: Some(token.to_owned()),
            basic: BasicAuthConfig {
                username: Some("user".to_owned()),
                password: Some("password".to_owned()),
            },
            ..SecurityConfig::default()
        };
        assert_eq!(
            config_with_basic.validate(),
            Err("the bearer token must not be empty or whitespace-only"),
            "a complete Basic authenticator must not preserve blank token {token:?}"
        );

        let config_with_jwt = SecurityConfig {
            enabled: true,
            bearer_token: Some(token.to_owned()),
            jwt: JwtConfig {
                enabled: true,
                secret: Some("jwt-alternative-secret".to_owned()),
                ..JwtConfig::default()
            },
            ..SecurityConfig::default()
        };
        assert_eq!(
            config_with_jwt.validate(),
            Err("the bearer token must not be empty or whitespace-only"),
            "a complete JWT authenticator must not preserve blank token {token:?}"
        );
    }
}

#[test]
fn application_bearer_validation_preserves_valid_token_exactly() {
    let config = SecurityConfig {
        enabled: true,
        bearer_token: Some("valid-token-sentinel".to_owned()),
        ..SecurityConfig::default()
    };

    assert_eq!(config.validate(), Ok(()));
    assert_eq!(config.bearer_token.as_deref(), Some("valid-token-sentinel"));

    let jwt_alternative = SecurityConfig {
        enabled: true,
        jwt: JwtConfig {
            enabled: true,
            secret: Some("jwt-alternative-secret".to_owned()),
            ..JwtConfig::default()
        },
        ..SecurityConfig::default()
    };
    assert_eq!(jwt_alternative.validate(), Ok(()));
}

#[test]
fn validation_errors_do_not_echo_a_configured_bearer_token() {
    let config = SecurityConfig {
        enabled: true,
        bearer_token: Some("validation-bearer-secret-sentinel".to_owned()),
        basic: BasicAuthConfig {
            username: Some(String::new()),
            password: Some("basic-password".to_owned()),
        },
        ..SecurityConfig::default()
    };

    let error = config
        .validate()
        .expect_err("invalid Basic configuration should fail validation");
    let diagnostic = format!("{error} {config:?}");
    assert_eq!(error, "basic credentials must not be empty");
    assert!(!diagnostic.contains("validation-bearer-secret-sentinel"));
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
