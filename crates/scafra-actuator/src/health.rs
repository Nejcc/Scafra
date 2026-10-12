use crate::{registry::HealthRegistry, HealthConfig};
use std::{
    any::Any,
    cell::Cell,
    future::Future,
    panic::{catch_unwind, AssertUnwindSafe},
    pin::Pin,
    sync::Once,
    task::{Context, Poll},
    time::Duration,
};

/// Async future returned by the legacy [`crate::HealthCheckRegistration`]
/// contract. It remains public for source compatibility.
///
/// A future must not block synchronously while being polled. The configured
/// timeout drops timed-out futures, which cancels cooperative async work but
/// cannot undo external effects or work detached elsewhere. Unwinding panics
/// while creating or polling a check fail closed without invoking the process
/// panic hook for that check. Other panics continue through the previously
/// installed hook. A later process-level call to `set_hook` can replace this
/// wrapper. With `panic=abort`, the process terminates instead.
pub type HealthCheckFuture = Pin<Box<dyn Future<Output = bool> + Send + 'static>>;

static INSTALL_HEALTH_CHECK_PANIC_HOOK: Once = Once::new();

// A thread-local guard keeps a check panic from suppressing another worker's
// panic report while this future is being polled.
thread_local! {
    static IN_HEALTH_CHECK: Cell<bool> = const { Cell::new(false) };
}

struct HealthCheckPanicHookGuard {
    was_in_health_check: bool,
}

impl HealthCheckPanicHookGuard {
    fn new() -> Self {
        INSTALL_HEALTH_CHECK_PANIC_HOOK.call_once(|| {
            let previous_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |panic_info| {
                if !IN_HEALTH_CHECK.with(Cell::get) {
                    previous_hook(panic_info);
                }
            }));
        });

        let was_in_health_check = IN_HEALTH_CHECK.with(|flag| flag.replace(true));
        Self {
            was_in_health_check,
        }
    }
}

impl Drop for HealthCheckPanicHookGuard {
    fn drop(&mut self) {
        IN_HEALTH_CHECK.with(|flag| flag.set(self.was_in_health_check));
    }
}

struct CatchUnwind {
    future: HealthCheckFuture,
}

impl Future for CatchUnwind {
    type Output = Result<bool, Box<dyn Any + Send + 'static>>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let result = {
            let _panic_hook_guard = HealthCheckPanicHookGuard::new();
            catch_unwind(AssertUnwindSafe(|| this.future.as_mut().poll(cx)))
        };
        match result {
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
        let future_result = {
            let _panic_hook_guard = HealthCheckPanicHookGuard::new();
            catch_unwind(AssertUnwindSafe(|| (registration.check)()))
        };
        let future = match future_result {
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
