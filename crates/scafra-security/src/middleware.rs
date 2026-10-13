//! Axum router middleware and HTTP authentication/authorization responses.

use crate::{
    authentication::authenticate,
    config::SecurityConfig,
    jwks::JwksCache,
    policy::{matching_policy, ResolvedPolicy, RouteAuthorizationMetadata},
};
use axum::{
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Router,
};
use std::sync::Arc;

/// Applies application-wide security to an application router.
pub fn layer(router: Router, config: SecurityConfig) -> Router {
    layer_with_route_policies(router, config, Vec::new())
}

/// Applies application-wide and generated route-level authorization policies.
pub fn layer_with_route_policies(
    router: Router,
    config: SecurityConfig,
    policies: Vec<RouteAuthorizationMetadata>,
) -> Router {
    if !config.enabled && policies.is_empty() {
        return router;
    }
    let state = SecurityState {
        config,
        policies: Arc::new(policies),
        jwks: JwksCache::new(),
    };
    router.layer(axum::middleware::from_fn(move |request, next| {
        let state = state.clone();
        async move { authorize_request(state, request, next).await }
    }))
}

#[derive(Clone)]
struct SecurityState {
    config: SecurityConfig,
    policies: Arc<Vec<RouteAuthorizationMetadata>>,
    jwks: JwksCache,
}

async fn authorize_request(
    state: SecurityState,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let path = request.uri().path();
    let policy = matching_policy(&state.policies, request.method(), path);
    let is_public = policy.is_some_and(ResolvedPolicy::is_public);

    if is_public || state.config.is_permitted(path) && policy.is_none() {
        return next.run(request).await;
    }

    let globally_protected = state.config.enabled && !state.config.is_permitted(path);
    let route_protected = policy.is_some_and(ResolvedPolicy::is_protected);
    if !globally_protected && !route_protected {
        return next.run(request).await;
    }

    let mut authorization_values = request
        .headers()
        .get_all(axum::http::header::AUTHORIZATION)
        .iter();
    let authorization = match (authorization_values.next(), authorization_values.next()) {
        (Some(_), Some(_)) => return denied(&state, StatusCode::UNAUTHORIZED),
        (Some(value), None) => value.to_str().ok(),
        (None, _) => None,
    };
    let Some(principal) = authenticate(&state.config, &state.jwks, authorization).await else {
        return denied(&state, StatusCode::UNAUTHORIZED);
    };

    let policy = policy.unwrap_or_default();
    let mut scopes = policy
        .controller_scopes
        .iter()
        .chain(policy.route_scopes.iter())
        .copied()
        .chain(
            principal
                .is_jwt
                .then_some(state.config.jwt.required_scopes.iter().map(String::as_str))
                .into_iter()
                .flatten(),
        );
    let authorized = policy
        .controller_role_groups
        .iter()
        .chain(policy.route_role_groups.iter())
        .all(|group| group.iter().any(|role| principal.roles.contains(*role)))
        && scopes.all(|scope| principal.scopes.contains(scope));
    if !authorized {
        return denied(&state, StatusCode::FORBIDDEN);
    }

    next.run(request).await
}

fn denied(state: &SecurityState, status: StatusCode) -> Response {
    if state.config.hide_unauthorized {
        StatusCode::NOT_FOUND.into_response()
    } else if status == StatusCode::UNAUTHORIZED {
        let challenge = if state.config.jwt.enabled || state.config.bearer_token.is_some() {
            "Bearer"
        } else {
            "Basic realm=\"Scafra\""
        };
        (status, [("www-authenticate", challenge)]).into_response()
    } else {
        status.into_response()
    }
}
