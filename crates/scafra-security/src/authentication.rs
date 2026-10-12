//! Bearer and Basic credential parsing and principal construction.

use crate::{config::SecurityConfig, jwks::JwksCache, jwt};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::collections::HashSet;

#[derive(Debug, Default)]
pub(super) struct Principal {
    pub(super) roles: HashSet<String>,
    pub(super) scopes: HashSet<String>,
    pub(super) is_jwt: bool,
}

pub(super) async fn authenticate(
    config: &SecurityConfig,
    jwks: &JwksCache,
    authorization: Option<&str>,
) -> Option<Principal> {
    let authorization = authorization?;
    let (scheme, credential) = authorization.split_once(' ')?;
    if credential.is_empty() || credential.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return None;
    }

    if scheme.eq_ignore_ascii_case("bearer") {
        if config.jwt.enabled {
            return jwt::validate_jwt(config, jwks, credential).await;
        }
        if config.bearer_token.as_deref() == Some(credential) {
            return Some(Principal::default());
        }
        return None;
    }

    if scheme.eq_ignore_ascii_case("basic") {
        let decoded = STANDARD.decode(credential).ok()?;
        let credentials = String::from_utf8(decoded).ok()?;
        let (username, password) = credentials.split_once(':')?;
        if config.basic.username.as_deref() == Some(username)
            && config.basic.password.as_deref() == Some(password)
        {
            return Some(Principal::default());
        }
    }
    None
}
