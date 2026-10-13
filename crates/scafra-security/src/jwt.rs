//! JWT signature, standard claim, role, and scope validation.

use crate::{
    authentication::Principal,
    config::SecurityConfig,
    jwks::{self, JwksCache},
};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde_json::Value;

pub(super) async fn validate_jwt(
    security: &SecurityConfig,
    jwks_cache: &JwksCache,
    token: &str,
) -> Option<Principal> {
    let config = &security.jwt;
    crate::config::validate_jwt_config(config).ok()?;
    let header = decode_header(token).ok()?;
    let key = if let Some(secret) = config.secret.as_deref() {
        if header.alg != Algorithm::HS256 {
            return None;
        }
        DecodingKey::from_secret(secret.as_bytes())
    } else {
        // Remote JWKS validation has a deliberately fixed RSA algorithm set.
        // Never pass a token-selected algorithm through without this check.
        if !matches!(
            header.alg,
            Algorithm::RS256 | Algorithm::RS384 | Algorithm::RS512
        ) {
            return None;
        }
        let kid = header.kid.as_deref()?;
        let key = jwks::get_key(jwks_cache, config, kid).await?;
        if key.kty != "RSA"
            || key
                .alg
                .as_deref()
                .is_some_and(|algorithm| algorithm != algorithm_name(header.alg))
            || key
                .public_key_use
                .as_deref()
                .is_some_and(|key_use| key_use != "sig")
            || key
                .key_ops
                .as_ref()
                .is_some_and(|operations| !operations.iter().any(|operation| operation == "verify"))
        {
            return None;
        }
        let (n, e) = (key.n.as_deref()?, key.e.as_deref()?);
        DecodingKey::from_rsa_components(n, e).ok()?
    };

    let mut validation = Validation::new(header.alg);
    validation.set_required_spec_claims(&["exp"]);
    validation.validate_nbf = true;
    if let Some(issuer) = config.issuer_uri.as_deref() {
        validation.set_issuer(&[issuer]);
    }
    if !config.audiences.is_empty() {
        validation.set_audience(&config.audiences);
    }
    let data = decode::<Value>(token, &key, &validation).ok()?;
    Some(principal_from_claims(&data.claims))
}

fn algorithm_name(algorithm: Algorithm) -> &'static str {
    match algorithm {
        Algorithm::RS256 => "RS256",
        Algorithm::RS384 => "RS384",
        Algorithm::RS512 => "RS512",
        _ => "",
    }
}

fn principal_from_claims(claims: &Value) -> Principal {
    let mut principal = Principal {
        is_jwt: true,
        ..Principal::default()
    };
    if let Some(roles) = claims.get("roles").and_then(Value::as_array) {
        principal.roles.extend(roles.iter().filter_map(|value| {
            value
                .as_str()
                .filter(|role| !role.is_empty())
                .map(str::to_owned)
        }));
    }
    if let Some(role) = claims.get("role") {
        match role {
            Value::String(role) if !role.is_empty() => {
                principal.roles.insert(role.clone());
            }
            Value::Array(roles) => principal.roles.extend(roles.iter().filter_map(|value| {
                value
                    .as_str()
                    .filter(|role| !role.is_empty())
                    .map(str::to_owned)
            })),
            _ => {}
        }
    }
    if let Some(scope) = claims.get("scope").and_then(Value::as_str) {
        principal
            .scopes
            .extend(scope.split_whitespace().map(str::to_owned));
    }
    if let Some(scopes) = claims.get("scp").and_then(Value::as_array) {
        principal.scopes.extend(scopes.iter().filter_map(|value| {
            value
                .as_str()
                .filter(|scope| !scope.is_empty())
                .map(str::to_owned)
        }));
    }
    principal
}
