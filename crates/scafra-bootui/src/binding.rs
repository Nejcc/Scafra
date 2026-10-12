use axum::{response::IntoResponse, Router};
use scafra_config::BootUiConfig;
use scafra_security::SecurityConfig;
use std::{
    fmt,
    net::{IpAddr, SocketAddr},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootUiPolicyError {
    InvalidHost,
    InvalidPort,
    InvalidPath,
    LoopbackRequired,
    AuthenticationRequired,
    InvalidSecurityConfiguration,
}

impl fmt::Display for BootUiPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidHost => "bootui.host must be an IPv4 or IPv6 address literal",
            Self::InvalidPort => "bootui.port must be greater than zero",
            Self::InvalidPath => "bootui.path must be a non-root absolute path",
            Self::LoopbackRequired => {
                "bootui.local_only=true requires a loopback IPv4 or IPv6 address"
            }
            Self::AuthenticationRequired => {
                "bootui.local_only=false requires security.enabled=true with valid credentials"
            }
            Self::InvalidSecurityConfiguration => {
                "BootUI security configuration is invalid; check the configured credentials"
            }
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for BootUiPolicyError {}

pub(crate) struct BindingPolicy {
    pub address: SocketAddr,
    pub protection: Protection,
}

pub(crate) enum Protection {
    LoopbackPeer,
    Authentication(Box<SecurityConfig>),
}

pub(crate) fn resolve(
    bootui: &BootUiConfig,
    security: &SecurityConfig,
) -> Result<BindingPolicy, BootUiPolicyError> {
    if bootui.port == 0 {
        return Err(BootUiPolicyError::InvalidPort);
    }

    let ip = bootui
        .host
        .parse::<IpAddr>()
        .map_err(|_| BootUiPolicyError::InvalidHost)?;
    let address = SocketAddr::new(ip, bootui.port);

    let protection = if bootui.local_only {
        if !ip.is_loopback() {
            return Err(BootUiPolicyError::LoopbackRequired);
        }
        Protection::LoopbackPeer
    } else {
        if !security.enabled {
            return Err(BootUiPolicyError::AuthenticationRequired);
        }
        if security.validate().is_err() {
            return Err(BootUiPolicyError::InvalidSecurityConfiguration);
        }

        // BootUI owns its routes and must not inherit a `permit_all` exception
        // from application routes. Authentication is mandatory for the whole
        // dashboard when local-only mode is disabled.
        let mut protected_security = security.clone();
        protected_security.enabled = true;
        protected_security.permit_all.clear();
        Protection::Authentication(Box::new(protected_security))
    };

    Ok(BindingPolicy {
        address,
        protection,
    })
}

pub(crate) fn protect_router(router: Router, protection: Protection) -> Router {
    match protection {
        Protection::LoopbackPeer => router.layer(axum::middleware::from_fn(require_loopback_peer)),
        Protection::Authentication(security) => scafra_security::layer(router, *security),
    }
}

async fn require_loopback_peer(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let loopback = request
        .extensions()
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .is_some_and(|axum::extract::ConnectInfo(address)| address.ip().is_loopback());
    if !loopback {
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }
    next.run(request).await
}
