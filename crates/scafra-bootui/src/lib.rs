//! Optional local operations dashboard for Scafra applications.
//!
//! Add this crate explicitly when a browser-based operations view is wanted.
//! Scafra core does not install or expose the dashboard by default.

mod binding;
mod dashboard;
mod snapshot;

pub use binding::BootUiPolicyError;
pub use scafra_config::BootUiConfig;
use scafra_config::{ConfigLoader, ScafraConfig};
use std::net::SocketAddr;

inventory::submit! {
    scafra_core::OptionalExtensionRegistration { name: "scafra-bootui", start }
}

/// Starts BootUI when it is enabled in `application.yml` or
/// `application.properties`.
///
/// The configuration snapshot is created once before the listener starts.
/// Invalid or unsafe bind policies are logged and prevent the listener from
/// starting.
pub fn start() {
    let config = match ConfigLoader::new().load::<ScafraConfig>() {
        Ok(config) => config,
        Err(_) => {
            tracing::error!(
                error_kind = "bootui_configuration",
                "BootUI configuration could not be loaded"
            );
            return;
        }
    };

    let prepared = match prepare(config) {
        Ok(Some(prepared)) => prepared,
        Ok(None) => return,
        Err(error) => {
            tracing::error!(
                error_kind = "bootui_policy",
                %error,
                "BootUI was not started; correct the BootUI bind and security configuration"
            );
            return;
        }
    };

    tokio::spawn(async move {
        let address = prepared.address;
        let path = prepared.path.clone();
        let listener = match tokio::net::TcpListener::bind(address).await {
            Ok(listener) => listener,
            Err(error) => {
                tracing::error!(
                    %address,
                    error_kind = "bootui_bind",
                    "BootUI could not bind its configured address: {error}"
                );
                return;
            }
        };
        tracing::info!(target: "scafra::startup", %address, path = %path, "Scafra BootUI started");
        let service = prepared
            .router
            .into_make_service_with_connect_info::<SocketAddr>();
        if let Err(error) = axum::serve(listener, service).await {
            tracing::error!(
                error_kind = "bootui_server",
                "BootUI stopped unexpectedly: {error}"
            );
        }
    });
}

/// Adds the BootUI dashboard to an existing Scafra router.
///
/// For compatibility, the dashboard settings are supplied separately and the
/// remaining application settings are loaded once from the normal
/// configuration sources. Prefer [`try_layer_with_config`] when the
/// application already has its complete configuration value.
///
/// A local-only dashboard checks the connecting peer address and rejects
/// requests when the server does not provide connection information. A
/// non-local dashboard is added only when Scafra authentication is configured.
/// Use Scafra's server adapter, which supplies peer information, or provide
/// equivalent `ConnectInfo<SocketAddr>` extensions in a custom Axum server.
pub fn layer(router: axum::Router, config: &BootUiConfig) -> axum::Router {
    if !config.enabled {
        return router;
    }

    let mut application_config = match ConfigLoader::new().load::<ScafraConfig>() {
        Ok(config) => config,
        Err(_) => {
            tracing::warn!(
                error_kind = "bootui_configuration",
                "BootUI is using default application link settings because configuration could not be loaded"
            );
            ScafraConfig::default()
        }
    };
    application_config.bootui = config.clone();
    match try_layer_with_config(router.clone(), &application_config) {
        Ok(router) => router,
        Err(error) => {
            tracing::error!(
                error_kind = "bootui_policy",
                %error,
                "BootUI routes were not added; correct the BootUI bind and security configuration"
            );
            router
        }
    }
}

/// Adds BootUI using a complete application configuration value.
///
/// This checked API is useful for custom configuration sources and returns a
/// policy error instead of silently omitting the dashboard. When
/// `local_only=true`, the configured BootUI address must be loopback and
/// requests from non-loopback peers are rejected. When `local_only=false`,
/// Scafra authentication must be enabled and is applied directly to every
/// BootUI route, regardless of `security.permit_all`.
pub fn try_layer_with_config(
    router: axum::Router,
    config: &ScafraConfig,
) -> Result<axum::Router, BootUiPolicyError> {
    if !config.bootui.enabled {
        return Ok(router);
    }

    let binding = binding::resolve(&config.bootui, &config.security)?;
    let snapshot = snapshot::BootUiSnapshot::from_config(config);
    let dashboard = dashboard::router(snapshot, binding.protection);
    let path = config.bootui.path.trim_end_matches('/');
    if path.is_empty() || !path.starts_with('/') {
        return Err(BootUiPolicyError::InvalidPath);
    }
    Ok(router.nest(path, dashboard))
}

struct PreparedBootUi {
    address: SocketAddr,
    path: String,
    router: axum::Router,
}

fn prepare(config: ScafraConfig) -> Result<Option<PreparedBootUi>, BootUiPolicyError> {
    if !config.bootui.enabled {
        return Ok(None);
    }

    let binding = binding::resolve(&config.bootui, &config.security)?;
    let path = config.bootui.path.trim_end_matches('/');
    if path.is_empty() || !path.starts_with('/') {
        return Err(BootUiPolicyError::InvalidPath);
    }

    let dashboard = dashboard::router(
        snapshot::BootUiSnapshot::from_config(&config),
        binding.protection,
    );
    Ok(Some(PreparedBootUi {
        address: binding.address,
        path: path.to_owned(),
        router: axum::Router::new().nest(path, dashboard),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_startup_policy_fails_before_a_listener_can_be_created() {
        let mut config = ScafraConfig::default();
        config.bootui.enabled = true;
        config.bootui.host = "0.0.0.0".to_owned();
        config.bootui.local_only = true;

        assert!(matches!(
            prepare(config),
            Err(BootUiPolicyError::LoopbackRequired)
        ));
    }

    #[test]
    fn startup_policy_accepts_ipv4_and_ipv6_loopback_addresses() {
        for host in ["127.0.0.1", "::1"] {
            let mut config = ScafraConfig::default();
            config.bootui.enabled = true;
            config.bootui.host = host.to_owned();

            let prepared = match prepare(config) {
                Ok(Some(prepared)) => prepared,
                Ok(None) => panic!("enabled BootUI should be prepared"),
                Err(error) => panic!("loopback host {host} should be accepted: {error}"),
            };
            assert!(prepared.address.ip().is_loopback());
        }
    }
}
