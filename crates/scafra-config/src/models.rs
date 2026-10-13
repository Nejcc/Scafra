use crate::sources::Config;
use scafra_actuator::ActuatorConfig;
use scafra_foundation::LoggingConfig;
use scafra_scheduler::SchedulerConfig;
use scafra_security::SecurityConfig;
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// Optional deadline, in seconds, for processing an HTTP request.
    /// `None` preserves Scafra's historical behavior without a deadline.
    #[serde(default, rename = "timeout", alias = "request_timeout_seconds")]
    pub request_timeout_seconds: Option<u64>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            request_timeout_seconds: None,
        }
    }
}

impl Config for ServerConfig {
    type Error = std::convert::Infallible;

    fn validate(&self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("configuration validation failed for {field}: {message}")]
pub struct ValidationError {
    pub field: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct StartupConfig {
    #[serde(default = "default_true")]
    pub banner: bool,
    #[serde(default = "default_true")]
    pub show_config: bool,
}

impl Default for StartupConfig {
    fn default() -> Self {
        Self {
            banner: true,
            show_config: true,
        }
    }
}

/// Configuration reserved for the optional `scafra-bootui` dependency.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct BootUiConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_bootui_host")]
    pub host: String,
    #[serde(default = "default_bootui_port")]
    pub port: u16,
    #[serde(default = "default_bootui_path")]
    pub path: String,
    #[serde(default = "default_true")]
    pub local_only: bool,
}

impl Default for BootUiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            host: default_bootui_host(),
            port: default_bootui_port(),
            path: default_bootui_path(),
            local_only: true,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ScafraConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub actuator: ActuatorConfig,
    #[serde(default)]
    pub startup: StartupConfig,
    #[serde(default)]
    pub scheduler: SchedulerConfig,
    #[serde(default)]
    pub security: SecurityConfig,
    #[serde(default)]
    pub bootui: BootUiConfig,
}

impl Config for ScafraConfig {
    type Error = ValidationError;

    fn validate(&self) -> Result<(), Self::Error> {
        if self.server.host.trim().is_empty() {
            return Err(ValidationError {
                field: "server.host",
                message: "must not be empty".to_owned(),
            });
        }
        if self.server.port == 0 {
            return Err(ValidationError {
                field: "server.port",
                message: "must be greater than zero".to_owned(),
            });
        }
        if matches!(self.server.request_timeout_seconds, Some(0)) {
            return Err(ValidationError {
                field: "server.request_timeout_seconds",
                message: "must be greater than zero when configured".to_owned(),
            });
        }
        if let Err(error) = self.scheduler.validate() {
            let (field, message) = match error {
                scafra_scheduler::SchedulerConfigError::UnknownTask => (
                    "scheduler.tasks",
                    "contains a task that is not registered".to_owned(),
                ),
                scafra_scheduler::SchedulerConfigError::ZeroInterval => (
                    "scheduler.tasks.interval_ms",
                    "must be greater than zero milliseconds".to_owned(),
                ),
                scafra_scheduler::SchedulerConfigError::IntervalTooLarge => (
                    "scheduler.tasks.interval_ms",
                    "must not exceed one year".to_owned(),
                ),
                scafra_scheduler::SchedulerConfigError::InvalidRegistration => (
                    "scheduler",
                    "contains a task registration with an invalid name or interval".to_owned(),
                ),
                scafra_scheduler::SchedulerConfigError::DuplicateRegistration => (
                    "scheduler",
                    "contains duplicate task registrations".to_owned(),
                ),
            };
            return Err(ValidationError { field, message });
        }
        if self.actuator.security.enabled
            && self
                .actuator
                .security
                .bearer_token
                .as_deref()
                .is_none_or(|token| token.trim().is_empty())
        {
            return Err(ValidationError {
                field: "actuator.security.bearer_token",
                message: "must be a non-blank bearer token when actuator security is enabled"
                    .to_owned(),
            });
        }
        if let Err(reason) = self.security.validate() {
            return Err(security_validation_error(reason));
        }
        if self.bootui.port == 0 {
            return Err(ValidationError {
                field: "bootui.port",
                message: "must be greater than zero".to_owned(),
            });
        }
        if !self.bootui.path.starts_with('/') {
            return Err(ValidationError {
                field: "bootui.path",
                message: "must start with '/'".to_owned(),
            });
        }
        Ok(())
    }

    fn validation_details(error: &Self::Error) -> Option<ValidationError> {
        Some(error.clone())
    }
}

fn security_validation_error(reason: &'static str) -> ValidationError {
    let (field, message) = match reason {
        "the bearer token must not be empty or whitespace-only" => (
            "security.bearer_token",
            "must not be empty or whitespace-only",
        ),
        "JWT requires secret, issuer_uri, or jwk_set_uri" => (
            "security.jwt",
            "requires secret, issuer_uri, or jwk_set_uri",
        ),
        "the JWT secret must not be empty" => (
            "security.jwt.secret",
            "must not be empty or whitespace-only",
        ),
        "JWT secret cannot be combined with remote JWKS configuration" => (
            "security.jwt",
            "secret cannot be combined with remote JWKS configuration",
        ),
        "JWT issuer_uri must be a valid HTTP(S) URL" => {
            ("security.jwt.issuer_uri", "must be a valid HTTP(S) URL")
        }
        "JWT jwk_set_uri must be a valid HTTP(S) URL" => {
            ("security.jwt.jwk_set_uri", "must be a valid HTTP(S) URL")
        }
        "JWT audiences must not contain empty or whitespace-only values" => (
            "security.jwt.audiences",
            "must not contain empty or whitespace-only values",
        ),
        _ => (
            "security",
            "credentials must be configured when security is enabled",
        ),
    };
    ValidationError {
        field,
        message: message.to_owned(),
    }
}

fn default_host() -> String {
    "127.0.0.1".to_owned()
}

fn default_port() -> u16 {
    8080
}

fn default_true() -> bool {
    true
}

fn default_bootui_host() -> String {
    "127.0.0.1".to_owned()
}

fn default_bootui_port() -> u16 {
    8091
}

fn default_bootui_path() -> String {
    "/bootui".to_owned()
}
