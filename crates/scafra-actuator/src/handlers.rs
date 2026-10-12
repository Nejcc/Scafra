use crate::{health, routing::ActuatorState, security};
use axum::{
    extract::Extension,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct InfoResponse {
    name: &'static str,
    version: &'static str,
}

async fn evaluate(state: &ActuatorState) -> bool {
    if !state.health_selection_valid {
        return false;
    }
    health::evaluate(&state.health_registry, &state.config.health).await
}

pub(crate) async fn health(
    headers: HeaderMap,
    Extension(state): Extension<Arc<ActuatorState>>,
) -> Response {
    if let Some(response) = security::protected(&headers, &state.config) {
        return response;
    }
    health_response(evaluate(&state).await)
}

pub(crate) async fn liveness(
    headers: HeaderMap,
    Extension(state): Extension<Arc<ActuatorState>>,
) -> Response {
    if let Some(response) = security::protected(&headers, &state.config) {
        return response;
    }
    health_response(true)
}

pub(crate) async fn readiness(
    headers: HeaderMap,
    Extension(state): Extension<Arc<ActuatorState>>,
) -> Response {
    if let Some(response) = security::protected(&headers, &state.config) {
        return response;
    }
    health_response(evaluate(&state).await)
}

fn health_response(up: bool) -> Response {
    (
        if up {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(HealthResponse {
            status: if up { "UP" } else { "DOWN" },
        }),
    )
        .into_response()
}

pub(crate) async fn info(
    headers: HeaderMap,
    Extension(state): Extension<Arc<ActuatorState>>,
) -> Response {
    if let Some(response) = security::protected(&headers, &state.config) {
        return response;
    }
    (
        StatusCode::OK,
        Json(InfoResponse {
            name: "scafra",
            version: env!("CARGO_PKG_VERSION"),
        }),
    )
        .into_response()
}

pub(crate) async fn metrics(
    headers: HeaderMap,
    Extension(state): Extension<Arc<ActuatorState>>,
) -> Response {
    if let Some(response) = security::protected(&headers, &state.config) {
        return response;
    }
    (StatusCode::OK, "# TYPE scafra_up gauge\nscafra_up 1\n").into_response()
}
