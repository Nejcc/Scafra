use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

use scafra::core::LifecycleHook;
use scafra::foundation::{ShutdownPolicy, ShutdownReason};
use scafra::{Application, ApplicationContext, ScheduledTaskConfig, Scheduler, SchedulerConfig};
use tokio::sync::Notify;

static EVENTS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
static TASK_RUNS: AtomicUsize = AtomicUsize::new(0);
static TASK_STARTED: Notify = Notify::const_new();

fn lifecycle_task() {
    TASK_RUNS.fetch_add(1, Ordering::SeqCst);
    EVENTS.lock().unwrap().push("task");
    TASK_STARTED.notify_one();
}
scafra::register_scheduled_task!("lifecycle_task", 60_000, lifecycle_task);

struct ShutdownHook;

impl LifecycleHook for ShutdownHook {
    fn name(&self) -> &'static str {
        "test_shutdown_hook"
    }

    fn on_start(&self, _: &ApplicationContext) -> Result<(), String> {
        Ok(())
    }

    fn on_shutdown(&self, _: &ApplicationContext) -> Result<(), String> {
        EVENTS.lock().unwrap().push("application_shutdown");
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn public_scheduler_shutdown_drains_before_application_shutdown() {
    EVENTS.lock().unwrap().clear();
    TASK_RUNS.store(0, Ordering::SeqCst);

    let mut application = Application::new(ApplicationContext::default()).with_hook(ShutdownHook);
    application.start().unwrap();

    let mut tasks = std::collections::BTreeMap::new();
    tasks.insert(
        "lifecycle_task".to_owned(),
        ScheduledTaskConfig {
            enabled: true,
            interval_ms: Some(10),
        },
    );
    let scheduler = Scheduler::try_start(&SchedulerConfig {
        enabled: true,
        tasks,
    })
    .unwrap();

    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_millis(10)).await;
    TASK_STARTED.notified().await;
    assert_eq!(TASK_RUNS.load(Ordering::SeqCst), 1);

    scheduler.shutdown().await;
    application
        .shutdown_with(
            ShutdownReason::ApplicationRequest,
            ShutdownPolicy::default(),
        )
        .unwrap();

    assert_eq!(
        *EVENTS.lock().unwrap(),
        vec!["task", "application_shutdown"]
    );
}
