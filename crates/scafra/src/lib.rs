//! Public facade for the Scafra framework.

use std::io::IsTerminal;

pub use scafra_actuator as actuator;
pub use scafra_actuator::register_health_check;
pub use scafra_actuator::{
    ActuatorConfig, ActuatorSecurity, EndpointSelection, HealthConfig, HealthRegistry,
    HealthRegistryError,
};
pub use scafra_config as config;
pub use scafra_config::{
    BootUiConfig, Config, ConfigError, ConfigLoader, ConfigProperties, LoggingConfig, Properties,
    ScafraConfig, SchedulerConfig, ServerConfig, StartupConfig, ValidationError,
};
pub use scafra_core as core;
pub use scafra_foundation as foundation;
pub use scafra_foundation::startup;
pub use scafra_foundation::{BacktraceMode, LogLevel, ShutdownPolicy, ShutdownReason};
pub use scafra_macros::{
    bean, component, controller, delete, get, logger, main, post, post_processor, put, repository,
    routes, service, Config,
};
pub use scafra_scheduler::{register_scheduled_task, ScheduledTaskConfig, Scheduler};
pub use scafra_security::{
    AuthorizationMode, AuthorizationPolicy, BasicAuthConfig, JwtConfig, SecurityConfig,
};
pub use scafra_web as web;
pub use tracing::{debug, error, info, trace, warn};

pub use scafra_core::{
    Application, ApplicationContext, Bean, BeanPostProcessor, ComponentKind, ComponentMetadata,
    GraphDescriptor, GraphEdgeDescriptor, GraphError, GraphNodeDescriptor, GraphNodeKind,
    GraphPhase, GraphPlan, ProviderFailure, ScafraError,
};
pub use scafra_web::{
    build_router, build_router_with_actuator_and_health_registry,
    build_router_with_actuator_and_health_registry_and_security, join_paths, run_on,
    run_on_with_log_level, serve_on, serve_on_with_actuator, serve_on_with_policy,
    serve_on_with_policy_and_actuator, serve_on_with_policy_and_actuator_and_security,
    serve_on_with_shutdown, shutdown_channel, AppError, ControllerPrefix, ControllerRegistration,
    ControllerRoutes, JsonBody, RouteAuthorizationMetadata, RouteMetadata, ServerError,
    ServerOutcome, ShutdownFuture, ShutdownHandle, ShutdownRequestError, WebError,
};

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    #[error(transparent)]
    Config(#[from] ConfigError),

    // Keep the payload for compatibility with callers that inspect the
    // variant, but never expose the configured value through Display. The
    // address may have come from an environment variable or secret-bearing
    // configuration source.
    #[error("invalid configured server address")]
    Address(String),

    #[error(transparent)]
    Core(#[from] scafra_core::ScafraError),

    #[error(transparent)]
    Graph(#[from] scafra_core::GraphError),

    #[error(transparent)]
    Web(#[from] WebError),
}

impl StartupError {
    /// Returns the stable public category used by generated startup logs.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Config(_) => "configuration",
            Self::Address(_) => "address",
            Self::Core(_) => "lifecycle",
            Self::Graph(_) => "dependency_graph",
            Self::Web(_) => "web",
        }
    }
}

#[doc(hidden)]
pub mod __private {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::{Child, Command},
        thread,
        time::{Duration, SystemTime},
    };

    pub use scafra_foundation as foundation;
    pub use tokio;
    pub use tracing;

    /// Runs the embedded debug supervisor used by ordinary `cargo run`.
    pub fn run_dev_supervisor(manifest_dir: &str, executable: PathBuf) -> i32 {
        let root = Path::new(manifest_dir);
        let mut snapshot = match dev_snapshot(root) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                eprintln!("Scafra dev reload could not watch the project: {error}");
                return 1;
            }
        };
        let mut child = match spawn_dev_child(&executable) {
            Ok(child) => child,
            Err(error) => {
                eprintln!("Scafra dev reload could not start the application: {error}");
                return 1;
            }
        };
        println!("Scafra dev reload: watching src, Cargo.toml, Cargo.lock, and resources");

        loop {
            thread::sleep(Duration::from_millis(500));
            let current = match dev_snapshot(root) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    eprintln!("Scafra dev reload could not inspect the project: {error}");
                    continue;
                }
            };
            if current == snapshot {
                if child.try_wait().ok().flatten().is_some() {
                    return 1;
                }
                continue;
            }
            snapshot = current;
            println!("\nScafra dev reload: watched file changed, rebuilding and restarting...\n");
            stop_dev_child(&mut child);
            let status = Command::new("cargo")
                .arg("build")
                .current_dir(root)
                .status();
            if !matches!(status, Ok(status) if status.success()) {
                eprintln!("Scafra dev reload: build failed; waiting for the next change");
                continue;
            }
            child = match spawn_dev_child(&executable) {
                Ok(child) => child,
                Err(error) => {
                    eprintln!("Scafra dev reload could not restart the application: {error}");
                    return 1;
                }
            };
        }
    }

    fn spawn_dev_child(executable: &Path) -> std::io::Result<Child> {
        Command::new(executable)
            .env("SCAFRA_DEV_CHILD", "1")
            .spawn()
    }

    fn stop_dev_child(child: &mut Child) {
        let _ = child.kill();
        let _ = child.wait();
    }

    fn dev_snapshot(root: &Path) -> std::io::Result<Vec<(PathBuf, Option<SystemTime>, u64)>> {
        let mut files = Vec::new();
        collect_dev_files(&root.join("src"), &mut files)?;
        for name in ["Cargo.toml", "Cargo.lock"] {
            let path = root.join(name);
            if path.exists() {
                let metadata = fs::metadata(&path)?;
                files.push((path, metadata.modified().ok(), metadata.len()));
            }
        }
        files.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(files)
    }

    fn collect_dev_files(
        path: &Path,
        files: &mut Vec<(PathBuf, Option<SystemTime>, u64)>,
    ) -> std::io::Result<()> {
        if !path.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if path
                .file_name()
                .is_some_and(|name| name == "target" || name == ".git")
            {
                continue;
            }
            if entry.file_type()?.is_dir() {
                collect_dev_files(&path, files)?;
            } else if entry.file_type()?.is_file() && is_dev_file(&path) {
                let metadata = fs::metadata(&path)?;
                files.push((path, metadata.modified().ok(), metadata.len()));
            }
        }
        Ok(())
    }

    fn is_dev_file(path: &Path) -> bool {
        matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("rs" | "yml" | "yaml" | "properties")
        )
    }

    /// Logs a bounded startup failure for the generated application entry point.
    ///
    /// This intentionally does not format the error: startup payloads can
    /// contain application-provided messages or secret-bearing configuration.
    pub fn log_startup_failure(error: &crate::StartupError) {
        match error {
            crate::StartupError::Web(scafra_web::WebError::Bind { address, source }) => {
                foundation::error!(
                    error_kind = error.kind(),
                    web_error_kind = "bind",
                    %address,
                    source = %source,
                    "application_startup_failed"
                );
            }
            crate::StartupError::Web(scafra_web::WebError::DuplicateRoute {
                method, path, ..
            }) => {
                foundation::error!(
                    error_kind = error.kind(),
                    web_error_kind = "duplicate_route",
                    %method,
                    %path,
                    "application_startup_failed"
                );
            }
            crate::StartupError::Web(scafra_web::WebError::Server(source)) => {
                foundation::error!(
                    error_kind = error.kind(),
                    web_error_kind = "server",
                    source = %source,
                    "application_startup_failed"
                );
            }
            _ => foundation::error!(error_kind = error.kind(), "application_startup_failed"),
        }
    }

    /// Logs cleanup failure metadata without formatting application-provided
    /// lifecycle messages.
    pub fn log_cleanup_failure(error: &crate::StartupError) {
        let failure_count = match error {
            crate::StartupError::Core(error) => error.lifecycle_cleanup_failures().len(),
            _ => 0,
        };
        foundation::error!(
            error_kind = "lifecycle_cleanup",
            failure_count,
            "application_cleanup_failed"
        );
    }
}

fn server_shutdown_reason(error: &ServerError) -> ShutdownReason {
    match error {
        ServerError::GracefulShutdownTimeout { reason, .. } => *reason,
        ServerError::Web(WebError::DuplicateRoute { .. } | WebError::Bind { .. }) => {
            ShutdownReason::StartupFailure
        }
        ServerError::Web(WebError::Server(source))
            if source.kind() == std::io::ErrorKind::InvalidInput =>
        {
            ShutdownReason::StartupFailure
        }
        ServerError::Web(WebError::Server(_)) => ShutdownReason::RuntimeFailure,
    }
}

/// Starts the standard Scafra server using the default local address.
pub async fn run() -> std::result::Result<(), StartupError> {
    run_inner(|_| Ok(None)).await
}

/// Starts Scafra with a router composed from the application's typed graph.
/// The builder runs after validated configuration loads and before lifecycle
/// startup, so providers can use configured values without global state.
pub async fn run_with_router<F>(build_router: F) -> std::result::Result<(), StartupError>
where
    F: FnOnce(&ScafraConfig) -> std::result::Result<axum::Router, StartupError>,
{
    run_inner(|config| build_router(config).map(Some)).await
}

async fn run_inner<F>(build_router: F) -> std::result::Result<(), StartupError>
where
    F: FnOnce(&ScafraConfig) -> std::result::Result<Option<axum::Router>, StartupError>,
{
    let loader = match std::env::var("SCAFRA_PROFILE") {
        Ok(profile) => ConfigLoader::new().profile(profile),
        Err(_) => ConfigLoader::new(),
    };
    let config = loader.load_validated::<ScafraConfig>()?;
    scafra_foundation::init_runtime_logging(&config.logging);
    let profile = loader
        .active_profile()
        .unwrap_or_else(|_| "default".to_owned());
    startup!(profile = %profile, root = %loader.configuration_root().display(), "configuration loaded");
    startup!(level = %config.logging.level, "logging initialized");
    let bootui_installed = inventory::iter::<scafra_core::OptionalExtensionRegistration>()
        .any(|extension| extension.name == "scafra-bootui");
    if config.bootui.enabled && !bootui_installed {
        scafra_foundation::warn!(
            "BootUI is enabled in configuration, but the scafra-bootui dependency is not installed"
        );
    } else {
        startup!(
            installed = bootui_installed,
            enabled = config.bootui.enabled,
            bind = %format!("{}:{}", config.bootui.host, config.bootui.port),
            path = %config.bootui.path,
            "BootUI status"
        );
    }
    if config.startup.banner {
        print_startup_banner(&config, &loader);
    }
    let address = format!("{}:{}", config.server.host, config.server.port)
        .parse()
        .map_err(|error: std::net::AddrParseError| StartupError::Address(error.to_string()))?;
    let router = build_router(&config)?;
    let context = ApplicationContext::discover();
    startup!(
        components = context.metadata().len(),
        "application components discovered"
    );
    let mut application = Application::new(context);
    application.start()?;
    startup!("application lifecycle started");
    let scheduler = Scheduler::start(&config.scheduler);
    startup!(enabled = config.scheduler.enabled, "scheduler initialized");
    let policy = ShutdownPolicy::default();
    let (result, reason) = match if let Some(router) = router {
        scafra_web::serve_router_on_with_policy_and_actuator_and_security_and_request_timeout(
            address,
            policy,
            config.actuator,
            config.security,
            config
                .server
                .request_timeout_seconds
                .map(std::time::Duration::from_secs),
            router,
        )
        .await
    } else {
        scafra_web::serve_on_with_policy_and_actuator_and_security_and_request_timeout(
            address,
            policy,
            config.actuator,
            config.security,
            config
                .server
                .request_timeout_seconds
                .map(std::time::Duration::from_secs),
        )
        .await
    } {
        Ok(outcome) => (Ok(()), outcome.reason()),
        Err(error) => {
            let reason = server_shutdown_reason(&error);
            (Err(StartupError::from(error.into_web_error())), reason)
        }
    };
    scheduler.stop();
    let shutdown = application
        .shutdown_with(reason, policy)
        .map_err(StartupError::from);
    match (result, shutdown) {
        (Err(error), Err(cleanup)) => {
            __private::log_cleanup_failure(&cleanup);
            Err(error)
        }
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn print_startup_banner(config: &ScafraConfig, loader: &ConfigLoader) {
    let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    if !config.startup.show_config {
        println!(
            "\n  {} {}\n",
            paint("Scafra", "36;1", color),
            paint(env!("CARGO_PKG_VERSION"), "37", color)
        );
        return;
    }
    let endpoints = config.actuator.enabled_endpoints();
    let endpoints = if endpoints.is_empty() {
        "none".to_owned()
    } else {
        endpoints.join(", ")
    };
    let profile = loader
        .active_profile()
        .unwrap_or_else(|_| "default".to_owned());
    let logo = [
        "  ███████╗███╗   ███╗██████╗ ███████╗██████╗",
        "  ██╔════╝████╗ ████║██╔══██╗██╔════╝██╔══██╗",
        "  █████╗  ██╔████╔██║██████╔╝█████╗  ██████╔╝",
        "  ██╔══╝  ██║╚██╔╝██║██╔══██╗██╔══╝  ██╔══██╗",
        "  ███████╗██║ ╚═╝ ██║██████╔╝███████╗██║  ██║",
        "  ╚══════╝╚═╝     ╚═╝╚═════╝ ╚══════╝╚═╝  ╚═╝",
    ];
    let logo = logo
        .iter()
        .enumerate()
        .map(|(index, line)| paint(line, if index % 2 == 0 { "38;5;208" } else { "33" }, color))
        .collect::<Vec<_>>()
        .join("\n");
    let label = |value: &str| paint(value, "38;5;208;1", color);
    println!(
        "\n{}\n\n  {} {}\n  {}       {}\n  {}       {}\n  {}       {}\n  {}      {}\n  {}     {}\n",
        logo,
        label("Scafra"),
        paint(env!("CARGO_PKG_VERSION"), "37", color),
        label("server"),
        paint(
            &format!("http://{}:{}", config.server.host, config.server.port),
            "37",
            color
        ),
        label("config"),
        paint(
            &loader.configuration_root().display().to_string(),
            "37",
            color
        ),
        label("profile"),
        paint(&profile, "35", color),
        label("log level"),
        paint(&config.logging.level.to_string(), "38;5;208", color),
        label("actuator"),
        paint(&endpoints, "37", color),
    );
}

fn paint(value: &str, code: &str, enabled: bool) -> String {
    if enabled {
        format!("\x1b[{}m{}\x1b[0m", code, value)
    } else {
        value.to_owned()
    }
}

/// Convenient imports for normal Scafra applications.
pub mod prelude {
    pub use crate::{
        bean, build_router, component, controller, debug, delete, error, get, info, logger, post,
        post_processor, put, register_health_check, register_scheduled_task, repository, routes,
        service, startup, trace, warn, ActuatorConfig, AppError, Application, ApplicationContext,
        AuthorizationMode, AuthorizationPolicy, BacktraceMode, Bean, BeanPostProcessor,
        BootUiConfig, ComponentKind, ComponentMetadata, Config, ConfigError, ConfigLoader,
        ConfigProperties, ControllerPrefix, ControllerRegistration, ControllerRoutes,
        GraphDescriptor, GraphEdgeDescriptor, GraphError, GraphNodeDescriptor, GraphNodeKind,
        GraphPhase, GraphPlan, HealthRegistry, HealthRegistryError, JsonBody, LogLevel,
        LoggingConfig, Properties, ProviderFailure, Result, RouteMetadata, ScafraConfig,
        ScafraError, SchedulerConfig, ServerConfig, ServerError, ServerOutcome, ShutdownPolicy,
        ShutdownReason, StartupError, ValidationError, WebError,
    };
    pub use axum::{
        extract::{Json, Path, Query},
        http::{HeaderMap, StatusCode},
        response::{IntoResponse, Response},
    };
}
