use std::{
    future::{Future, IntoFuture},
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Duration,
};

use scafra_foundation::{
    init_runtime_logging_with_directive, LoggingConfig, ShutdownPolicy, ShutdownReason,
};
use tokio::{net::TcpListener, sync::oneshot};

use crate::errors::{ServerError, WebError};

/// The successful result of a server lifecycle boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerOutcome {
    reason: ShutdownReason,
}

impl ServerOutcome {
    /// Returns the typed reason that ended HTTP serving.
    pub const fn reason(self) -> ShutdownReason {
        self.reason
    }
}

/// A one-shot application-owned request to end HTTP serving.
#[derive(Clone)]
pub struct ShutdownHandle {
    sender: Arc<Mutex<Option<oneshot::Sender<ShutdownReason>>>>,
}

impl std::fmt::Debug for ShutdownHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ShutdownHandle")
            .field(
                "requestable",
                &self.sender.lock().is_ok_and(|sender| sender.is_some()),
            )
            .finish()
    }
}

impl ShutdownHandle {
    /// Requests an application-owned shutdown.
    pub fn request(&self) -> Result<(), ShutdownRequestError> {
        self.request_with_reason(ShutdownReason::ApplicationRequest)
    }

    /// Requests shutdown with an explicit foundation reason.
    pub fn request_with_reason(&self, reason: ShutdownReason) -> Result<(), ShutdownRequestError> {
        let sender = self
            .sender
            .lock()
            .map_err(|_| ShutdownRequestError::ChannelUnavailable)?
            .take()
            .ok_or(ShutdownRequestError::AlreadyRequested)?;
        sender
            .send(reason)
            .map_err(|_| ShutdownRequestError::ReceiverDropped)
    }
}

/// The future paired with a [`ShutdownHandle`].
pub struct ShutdownFuture {
    receiver: oneshot::Receiver<ShutdownReason>,
}

impl std::fmt::Debug for ShutdownFuture {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ShutdownFuture")
            .finish_non_exhaustive()
    }
}

impl Future for ShutdownFuture {
    type Output = ShutdownReason;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.get_mut().receiver).poll(context) {
            Poll::Ready(Ok(reason)) => Poll::Ready(reason),
            // Dropping the request handle must not leave the server waiting
            // forever. It is an adapter failure, not an application request.
            Poll::Ready(Err(_)) => Poll::Ready(ShutdownReason::RuntimeFailure),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Failure to deliver an explicit shutdown request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ShutdownRequestError {
    #[error("the Scafra shutdown request was already sent")]
    AlreadyRequested,

    #[error("the Scafra server is no longer waiting for a shutdown request")]
    ReceiverDropped,

    #[error("the Scafra shutdown request channel is unavailable")]
    ChannelUnavailable,
}

/// Creates an explicit application-request shutdown channel.
pub fn shutdown_channel() -> (ShutdownHandle, ShutdownFuture) {
    let (sender, receiver) = oneshot::channel();
    (
        ShutdownHandle {
            sender: Arc::new(Mutex::new(Some(sender))),
        },
        ShutdownFuture { receiver },
    )
}

/// Runs the default Scafra server on 127.0.0.1:8080.
pub async fn run() -> Result<(), WebError> {
    run_on(
        "127.0.0.1:8080"
            .parse()
            .expect("default Scafra address is valid"),
    )
    .await
}

/// Runs the registered application on a caller-provided address.
pub async fn run_on(address: SocketAddr) -> Result<(), WebError> {
    run_on_with_log_level(address, "info").await
}

/// Runs the registered application using a textual tracing directive for
/// compatibility with the original Scafra web API.
pub async fn run_on_with_log_level(address: SocketAddr, log_level: &str) -> Result<(), WebError> {
    init_runtime_logging_with_directive(&LoggingConfig::default(), Some(log_level));
    serve_on(address).await
}

/// Serves the registered application without configuring a subscriber.
///
/// The standard facade initializes foundation logging before calling this
/// boundary. Applications composing Axum directly can use this function after
/// installing their own subscriber.
pub async fn serve_on(address: SocketAddr) -> Result<(), WebError> {
    serve_on_with_policy(address, ShutdownPolicy::default())
        .await
        .map(|_| ())
        .map_err(ServerError::into_web_error)
}

/// Serves the application with an explicit observation configuration.
pub async fn serve_on_with_actuator(
    address: SocketAddr,
    actuator: scafra_actuator::ActuatorConfig,
) -> Result<(), WebError> {
    serve_on_with_policy_and_actuator(address, ShutdownPolicy::default(), actuator)
        .await
        .map(|_| ())
        .map_err(ServerError::into_web_error)
}

/// Serves the registered application with the default Ctrl-C/SIGTERM signal
/// future and an explicit graceful-drain policy.
pub async fn serve_on_with_policy(
    address: SocketAddr,
    policy: ShutdownPolicy,
) -> Result<ServerOutcome, ServerError> {
    serve_on_with_signal(
        address,
        shutdown_signal(),
        policy,
        scafra_actuator::ActuatorConfig::default(),
        scafra_security::SecurityConfig::default(),
        None,
        None,
    )
    .await
}

/// Serves the registered application until an application-owned shutdown
/// future resolves to a typed foundation reason.
pub async fn serve_on_with_shutdown<F>(
    address: SocketAddr,
    shutdown: F,
    policy: ShutdownPolicy,
) -> Result<ServerOutcome, ServerError>
where
    F: Future<Output = ShutdownReason> + Send + 'static,
{
    serve_on_with_signal(
        address,
        async move { Ok(shutdown.await) },
        policy,
        scafra_actuator::ActuatorConfig::default(),
        scafra_security::SecurityConfig::default(),
        None,
        None,
    )
    .await
}

/// Serves the registered application with observation endpoints configured by
/// the application.
pub async fn serve_on_with_policy_and_actuator(
    address: SocketAddr,
    policy: ShutdownPolicy,
    actuator: scafra_actuator::ActuatorConfig,
) -> Result<ServerOutcome, ServerError> {
    serve_on_with_policy_and_actuator_and_security(
        address,
        policy,
        actuator,
        scafra_security::SecurityConfig::default(),
    )
    .await
}

pub async fn serve_on_with_policy_and_actuator_and_security(
    address: SocketAddr,
    policy: ShutdownPolicy,
    actuator: scafra_actuator::ActuatorConfig,
    security: scafra_security::SecurityConfig,
) -> Result<ServerOutcome, ServerError> {
    serve_on_with_policy_and_actuator_and_security_and_request_timeout(
        address, policy, actuator, security, None,
    )
    .await
}

/// Serves the registered application with an optional request-processing
/// deadline, actuator and security configuration.
pub async fn serve_on_with_policy_and_actuator_and_security_and_request_timeout(
    address: SocketAddr,
    policy: ShutdownPolicy,
    actuator: scafra_actuator::ActuatorConfig,
    security: scafra_security::SecurityConfig,
    request_timeout: Option<Duration>,
) -> Result<ServerOutcome, ServerError> {
    serve_on_with_policy_and_actuator_and_security_and_request_timeout_and_shutdown_callback(
        address,
        policy,
        actuator,
        security,
        request_timeout,
        |_| {},
    )
    .await
}

/// Serves the registered application and calls `on_shutdown_requested` as
/// soon as a shutdown signal arrives, before HTTP request draining begins.
pub async fn serve_on_with_policy_and_actuator_and_security_and_request_timeout_and_shutdown_callback<
    F,
>(
    address: SocketAddr,
    policy: ShutdownPolicy,
    actuator: scafra_actuator::ActuatorConfig,
    security: scafra_security::SecurityConfig,
    request_timeout: Option<Duration>,
    on_shutdown_requested: F,
) -> Result<ServerOutcome, ServerError>
where
    F: FnOnce(ShutdownReason) + Send + 'static,
{
    serve_on_with_signal(
        address,
        shutdown_with_callback(shutdown_signal(), on_shutdown_requested),
        policy,
        actuator,
        security,
        request_timeout,
        None,
    )
    .await
}

/// Serves a router assembled from typed dependency injection while retaining
/// Scafra's normal shutdown and middleware configuration.
pub async fn serve_router_on_with_policy_and_actuator_and_security_and_request_timeout(
    address: SocketAddr,
    policy: ShutdownPolicy,
    actuator: scafra_actuator::ActuatorConfig,
    security: scafra_security::SecurityConfig,
    request_timeout: Option<Duration>,
    router: axum::Router,
) -> Result<ServerOutcome, ServerError> {
    serve_router_on_with_policy_and_actuator_and_security_and_request_timeout_and_shutdown_callback(
        address,
        policy,
        actuator,
        security,
        request_timeout,
        router,
        |_| {},
    )
    .await
}

/// Serves a typed application router and notifies lifecycle owners as soon as
/// shutdown begins, before Axum drains active requests.
pub async fn serve_router_on_with_policy_and_actuator_and_security_and_request_timeout_and_shutdown_callback<
    F,
>(
    address: SocketAddr,
    policy: ShutdownPolicy,
    actuator: scafra_actuator::ActuatorConfig,
    security: scafra_security::SecurityConfig,
    request_timeout: Option<Duration>,
    router: axum::Router,
    on_shutdown_requested: F,
) -> Result<ServerOutcome, ServerError>
where
    F: FnOnce(ShutdownReason) + Send + 'static,
{
    serve_on_with_signal(
        address,
        shutdown_with_callback(shutdown_signal(), on_shutdown_requested),
        policy,
        actuator,
        security,
        request_timeout,
        Some(router),
    )
    .await
}

async fn shutdown_with_callback<F, C>(shutdown: F, callback: C) -> Result<ShutdownReason, WebError>
where
    F: Future<Output = Result<ShutdownReason, WebError>> + Send + 'static,
    C: FnOnce(ShutdownReason) + Send + 'static,
{
    let result = shutdown.await;
    callback(
        result
            .as_ref()
            .copied()
            .unwrap_or(ShutdownReason::RuntimeFailure),
    );
    result
}

async fn serve_on_with_signal<F>(
    address: SocketAddr,
    shutdown: F,
    policy: ShutdownPolicy,
    actuator: scafra_actuator::ActuatorConfig,
    security: scafra_security::SecurityConfig,
    request_timeout: Option<Duration>,
    supplied_router: Option<axum::Router>,
) -> Result<ServerOutcome, ServerError>
where
    F: Future<Output = Result<ShutdownReason, WebError>> + Send + 'static,
{
    let actuator_endpoints = actuator.enabled_endpoints().join(",");
    scafra_foundation::startup!(
        actuator = %actuator_endpoints,
        security = security.enabled,
        "web configuration initialized"
    );
    let router = match supplied_router {
        Some(router) => router,
        None => crate::routing::build_router_with_timeout(&actuator, &security, request_timeout)
            .map_err(ServerError::from)?,
    };
    scafra_foundation::startup!("application routes built");
    let listener = TcpListener::bind(address)
        .await
        .map_err(|source| ServerError::from(WebError::Bind { address, source }))?;
    scafra_foundation::startup!(%address, "server socket bound");
    for extension in inventory::iter::<scafra_core::OptionalExtensionRegistration> {
        (extension.start)();
    }
    scafra_foundation::startup!(%address, "Scafra server started");

    serve_listener(listener, router, shutdown, policy).await
}

pub(crate) async fn serve_listener<F>(
    listener: TcpListener,
    router: axum::Router,
    shutdown: F,
    policy: ShutdownPolicy,
) -> Result<ServerOutcome, ServerError>
where
    F: Future<Output = Result<ShutdownReason, WebError>> + Send + 'static,
{
    let (trigger_sender, mut trigger_receiver) = oneshot::channel();
    let graceful_shutdown = async move {
        let trigger = shutdown.await;
        let _ = trigger_sender.send(trigger);
    };
    let service = router.into_make_service_with_connect_info::<SocketAddr>();
    let server = axum::serve(listener, service)
        .with_graceful_shutdown(graceful_shutdown)
        .into_future();
    tokio::pin!(server);

    let trigger = tokio::select! {
        // Axum completes its serving future as soon as the graceful-shutdown
        // future resolves. Prefer the trigger channel when both become ready
        // in the same poll, so a successful shutdown is not mistaken for an
        // unsolicited server exit.
        biased;
        trigger = &mut trigger_receiver => trigger.unwrap_or_else(|_| {
            Err(WebError::Server(std::io::Error::other(
                "Scafra shutdown trigger stopped unexpectedly",
            )))
        }),
        result = &mut server => {
            return match result {
                Ok(()) => Err(ServerError::from(WebError::Server(std::io::Error::other(
                    "Scafra server stopped before a shutdown request",
                )))),
                Err(source) => Err(ServerError::from(WebError::Server(source))),
            };
        }
    };

    let (reason, trigger_error) = match trigger {
        Ok(reason) => (reason, None),
        Err(error) => (ShutdownReason::RuntimeFailure, Some(error)),
    };
    scafra_foundation::info!(?reason, "Scafra server shutting down");

    let server_result = if policy.force_after_grace() {
        match tokio::time::timeout(policy.grace_period(), &mut server).await {
            Ok(result) => result,
            Err(_) => {
                return Err(ServerError::GracefulShutdownTimeout { reason, policy });
            }
        }
    } else {
        (&mut server).await
    };

    match server_result {
        Err(source) => Err(ServerError::from(WebError::Server(source))),
        Ok(()) => match trigger_error {
            Some(error) => Err(ServerError::from(error)),
            None => Ok(ServerOutcome { reason }),
        },
    }
}

async fn shutdown_signal() -> Result<ShutdownReason, WebError> {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .map(|_| ShutdownReason::Signal)
            .map_err(|error| signal_error("Ctrl-C", error))
    };

    #[cfg(unix)]
    let terminate = async {
        let mut signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .map_err(|error| signal_error("SIGTERM", error))?;
        signal
            .recv()
            .await
            .map(|_| ShutdownReason::Signal)
            .ok_or_else(|| {
                WebError::Server(std::io::Error::other(
                    "SIGTERM handler stopped before receiving a signal",
                ))
            })
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<Result<ShutdownReason, WebError>>();

    tokio::select! {
        result = ctrl_c => result,
        result = terminate => result,
    }
}

fn signal_error(name: &str, error: std::io::Error) -> WebError {
    WebError::Server(std::io::Error::new(
        error.kind(),
        format!("failed to install {name} handler: {error}"),
    ))
}

#[cfg(test)]
mod shutdown_callback_tests {
    use super::*;
    use axum::{extract::State, routing::get, Router};
    use std::{sync::Arc, time::Duration};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::{oneshot, Notify},
    };

    #[derive(Clone)]
    struct DrainState {
        started: Arc<Notify>,
        release: Arc<Notify>,
    }

    async fn blocked_request(State(state): State<DrainState>) -> &'static str {
        state.started.notify_one();
        state.release.notified().await;
        "drained"
    }

    async fn connect_when_ready(address: SocketAddr) -> TcpStream {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match TcpStream::connect(address).await {
                    Ok(stream) => return stream,
                    Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
                        tokio::task::yield_now().await;
                    }
                    Err(error) => panic!("server connection failed: {error}"),
                }
            }
        })
        .await
        .expect("server should bind within the test deadline")
    }

    #[tokio::test]
    async fn shutdown_callback_runs_before_http_request_drain_completes() {
        let reservation = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test should bind loopback");
        let address = reservation.local_addr().expect("listener address exists");
        drop(reservation);

        let state = DrainState {
            started: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
        };
        let router = Router::new()
            .route("/drain", get(blocked_request))
            .with_state(state.clone());
        let (trigger_sender, trigger_receiver) = oneshot::channel();
        let shutdown = async move {
            Ok(trigger_receiver
                .await
                .unwrap_or(ShutdownReason::RuntimeFailure))
        };
        let (callback_sender, callback_receiver) = oneshot::channel();
        let callback = move |reason| {
            let _ = callback_sender.send(reason);
        };
        let server = tokio::spawn(serve_on_with_signal(
            address,
            shutdown_with_callback(shutdown, callback),
            ShutdownPolicy::new(Duration::from_secs(2), true),
            scafra_actuator::ActuatorConfig::default(),
            scafra_security::SecurityConfig::default(),
            None,
            Some(router),
        ));

        let mut connection = connect_when_ready(address).await;
        connection
            .write_all(b"GET /drain HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .expect("request should be written");
        state.started.notified().await;
        trigger_sender
            .send(ShutdownReason::Signal)
            .expect("server should still await its shutdown signal");

        assert_eq!(callback_receiver.await, Ok(ShutdownReason::Signal));
        assert!(
            !server.is_finished(),
            "the callback should run while an active request is still draining"
        );

        state.release.notify_one();
        let mut response = Vec::new();
        connection
            .read_to_end(&mut response)
            .await
            .expect("drained response should be readable");
        assert!(String::from_utf8_lossy(&response).ends_with("drained"));
        server
            .await
            .expect("server task should join")
            .expect("request should drain successfully");
    }
}
