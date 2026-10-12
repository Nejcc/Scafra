use crate::{registry::HealthRegistry, HealthConfig};
use std::{
    any::Any,
    future::Future,
    panic::{catch_unwind, AssertUnwindSafe},
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

/// Async future returned by the legacy [`crate::HealthCheckRegistration`]
/// contract. It remains public for source compatibility.
///
/// A future must not block synchronously while being polled. The configured
/// timeout drops timed-out futures, which cancels cooperative async work but
/// cannot undo external effects or work detached elsewhere. Unwinding panics
/// while creating or polling a check fail closed; `panic=abort` terminates the
/// process.
pub type HealthCheckFuture = Pin<Box<dyn Future<Output = bool> + Send + 'static>>;

struct CatchUnwind {
    future: HealthCheckFuture,
}

impl Future for CatchUnwind {
    type Output = Result<bool, Box<dyn Any + Send + 'static>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match catch_unwind(AssertUnwindSafe(|| this.future.as_mut().poll(cx))) {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(healthy)) => Poll::Ready(Ok(healthy)),
            Err(panic) => Poll::Ready(Err(panic)),
        }
    }
}

pub(crate) async fn evaluate(registry: &HealthRegistry, config: &HealthConfig) -> bool {
    for registration in registry.selected_checks(config) {
        // Async checks let timeout drop their work. A synchronous check moved
        // to spawn_blocking could not be stopped and could exhaust that pool.
        let future = match catch_unwind(AssertUnwindSafe(|| (registration.check)())) {
            Ok(future) => future,
            Err(_) => {
                tracing::error!(health_check = registration.name, "health check panicked");
                return false;
            }
        };
        let result = tokio::time::timeout(
            Duration::from_millis(config.check_timeout_ms),
            CatchUnwind { future },
        )
        .await;
        match result {
            Ok(Ok(true)) => {}
            Ok(Err(_)) => {
                tracing::error!(health_check = registration.name, "health check panicked");
                return false;
            }
            Ok(Ok(false)) | Err(_) => return false,
        }
    }
    true
}
