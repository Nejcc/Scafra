use crate::ActuatorConfig;
use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

pub(crate) fn protected(headers: &HeaderMap, config: &ActuatorConfig) -> Option<Response> {
    if !config.security.enabled {
        return None;
    }
    let Some(token) = config.security.bearer_token.as_deref() else {
        return Some(StatusCode::UNAUTHORIZED.into_response());
    };
    if token.trim().is_empty() {
        return Some(StatusCode::UNAUTHORIZED.into_response());
    }
    let valid = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|candidate| candidate == token);
    (!valid).then(|| StatusCode::UNAUTHORIZED.into_response())
}
