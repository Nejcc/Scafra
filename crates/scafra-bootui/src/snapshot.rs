use scafra_config::ScafraConfig;
use serde::Serialize;

/// The dashboard only needs the application address to build actuator links.
/// Keep this allow-list independent of the full configuration model so new
/// configuration and credential fields cannot leak into the endpoint.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct BootUiSnapshot {
    server: ServerSnapshot,
}

#[derive(Debug, Clone, Serialize)]
struct ServerSnapshot {
    host: String,
    port: u16,
}

impl BootUiSnapshot {
    pub(crate) fn from_config(config: &ScafraConfig) -> Self {
        Self {
            server: ServerSnapshot {
                host: config.server.host.clone(),
                port: config.server.port,
            },
        }
    }
}
