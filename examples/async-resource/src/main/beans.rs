use std::{
    error::Error,
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
};

use scafra::{bean, AsyncResource, ShutdownPolicy, ShutdownReason};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};
use tokio::{
    sync::{watch, Mutex as AsyncMutex, Notify},
    task::JoinHandle,
};

static EVENTS: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
pub(crate) static WORKER_STARTED: Notify = Notify::const_new();
pub(crate) static FAIL_WORKER: AtomicBool = AtomicBool::new(false);
pub(crate) static FAIL_CLIENT_SHUTDOWN: AtomicBool = AtomicBool::new(false);
pub(crate) static IGNORE_WORKER_CANCELLATION: AtomicBool = AtomicBool::new(false);
pub(crate) static DATABASE_OPEN: AtomicBool = AtomicBool::new(false);

fn event_log() -> &'static Mutex<Vec<&'static str>> {
    EVENTS.get_or_init(|| Mutex::new(Vec::new()))
}

fn record(event: &'static str) {
    if let Ok(mut events) = event_log().lock() {
        events.push(event);
    }
}

#[cfg(test)]
pub(crate) fn events() -> Vec<&'static str> {
    event_log()
        .lock()
        .map(|events| events.clone())
        .unwrap_or_default()
}

#[cfg(test)]
pub(crate) fn reset_for_test() {
    if let Ok(mut events) = event_log().lock() {
        events.clear();
    }
    FAIL_WORKER.store(false, Ordering::SeqCst);
    FAIL_CLIENT_SHUTDOWN.store(false, Ordering::SeqCst);
    IGNORE_WORKER_CANCELLATION.store(false, Ordering::SeqCst);
    DATABASE_OPEN.store(false, Ordering::SeqCst);
}

fn configured_failure(resource: &str) -> bool {
    std::env::var("ASYNC_RESOURCE_EXAMPLE_FAIL_RESOURCE").as_deref() == Ok(resource)
}

#[derive(Debug)]
pub enum ResourceError {
    Message(&'static str),
    Database(sqlx::Error),
}

impl ResourceError {
    fn message(message: &'static str) -> Self {
        Self::Message(message)
    }

    fn database(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl fmt::Display for ResourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Message(message) => formatter.write_str(message),
            Self::Database(error) => write!(formatter, "SQLite pool initialization failed: {error}"),
        }
    }
}

impl Error for ResourceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Message(_) => None,
            Self::Database(error) => Some(error),
        }
    }
}

pub struct DatabasePool {
    pool: SqlitePool,
}

impl AsyncResource for DatabasePool {
    type ShutdownError = ResourceError;

    async fn shutdown(
        &self,
        _reason: ShutdownReason,
        _policy: ShutdownPolicy,
    ) -> Result<(), Self::ShutdownError> {
        record("database.shutdown");
        self.pool.close().await;
        DATABASE_OPEN.store(false, Ordering::SeqCst);
        Ok(())
    }
}

pub struct ServiceClient {
    _pool: Arc<DatabasePool>,
}

impl AsyncResource for ServiceClient {
    type ShutdownError = ResourceError;

    async fn shutdown(
        &self,
        _reason: ShutdownReason,
        _policy: ShutdownPolicy,
    ) -> Result<(), Self::ShutdownError> {
        record("client.shutdown");
        if FAIL_CLIENT_SHUTDOWN.load(Ordering::SeqCst)
            || std::env::var("ASYNC_RESOURCE_EXAMPLE_FAIL_CLEANUP").as_deref() == Ok("client")
        {
            return Err(ResourceError::message(
                "configured client shutdown failure",
            ));
        }
        Ok(())
    }
}

struct TaskDropSignal;

impl Drop for TaskDropSignal {
    fn drop(&mut self) {
        record("worker.task_dropped");
    }
}

pub struct Worker {
    _client: Arc<ServiceClient>,
    cancellation: watch::Sender<bool>,
    join: AsyncMutex<Option<JoinHandle<()>>>,
}

impl AsyncResource for Worker {
    type ShutdownError = ResourceError;

    fn request_shutdown(&self, _reason: ShutdownReason) {
        self.cancellation.send_replace(true);
    }

    async fn shutdown(
        &self,
        _reason: ShutdownReason,
        policy: ShutdownPolicy,
    ) -> Result<(), Self::ShutdownError> {
        record("worker.shutdown");
        self.cancellation.send_replace(true);
        let mut join = self.join.lock().await;
        let Some(handle) = join.as_mut() else {
            return Ok(());
        };
        match tokio::time::timeout(policy.grace_period(), handle).await {
            Ok(Ok(())) => {
                join.take();
                Ok(())
            }
            Ok(Err(_)) => {
                join.take();
                Err(ResourceError::message("worker task failed while joining"))
            }
            Err(_) => {
                if let Some(handle) = join.as_ref() {
                    handle.abort();
                }
                if let Some(handle) = join.take() {
                    let _ = handle.await;
                }
                Err(ResourceError::message(
                    "worker task exceeded its shutdown grace period and was aborted",
                ))
            }
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(handle) = self.join.get_mut().as_ref() {
            handle.abort();
        }
    }
}

#[bean]
pub async fn database_pool() -> Result<DatabasePool, ResourceError> {
    record("database.initialize");
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .map_err(ResourceError::database)?;
    if configured_failure("database") {
        pool.close().await;
        return Err(ResourceError::message(
            "configured database initialization failure",
        ));
    }
    if let Err(error) = sqlx::query("SELECT 1").execute(&pool).await {
        pool.close().await;
        return Err(ResourceError::database(error));
    }
    DATABASE_OPEN.store(true, Ordering::SeqCst);
    Ok(DatabasePool { pool })
}

#[bean]
pub async fn service_client(pool: Arc<DatabasePool>) -> Result<ServiceClient, ResourceError> {
    record("client.initialize");
    tokio::task::yield_now().await;
    if configured_failure("client") {
        return Err(ResourceError::message(
            "configured client initialization failure",
        ));
    }
    sqlx::query("SELECT 1")
        .execute(&pool.pool)
        .await
        .map_err(ResourceError::database)?;
    Ok(ServiceClient { _pool: pool })
}

#[bean]
pub async fn worker(client: Arc<ServiceClient>) -> Result<Worker, ResourceError> {
    record("worker.initialize");
    tokio::task::yield_now().await;
    if FAIL_WORKER.load(Ordering::SeqCst) || configured_failure("worker") {
        return Err(ResourceError::message(
            "configured worker initialization failure",
        ));
    }

    let (cancellation, mut receiver) = watch::channel(false);
    let ignore_cancellation = IGNORE_WORKER_CANCELLATION.load(Ordering::SeqCst)
        || std::env::var("ASYNC_RESOURCE_EXAMPLE_IGNORE_WORKER_CANCELLATION").as_deref()
            == Ok("true");
    let join = tokio::spawn(async move {
        let _drop_signal = TaskDropSignal;
        record("worker.started");
        WORKER_STARTED.notify_one();
        if ignore_cancellation {
            std::future::pending::<()>().await;
        } else {
            loop {
                if *receiver.borrow() {
                    break;
                }
                if receiver.changed().await.is_err() {
                    break;
                }
            }
            record("worker.cancelled");
            record("worker.finished");
        }
    });

    Ok(Worker {
        _client: client,
        cancellation,
        join: AsyncMutex::new(Some(join)),
    })
}

