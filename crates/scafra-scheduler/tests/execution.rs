use std::{
    collections::BTreeMap,
    io::{self, Write},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
};

use scafra_scheduler::{ScheduledTaskConfig, Scheduler, SchedulerConfig, SchedulerError};
use tokio::sync::Notify;
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for LogBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for LogBuffer {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

static FIRST_RUNS: AtomicUsize = AtomicUsize::new(0);
static FIRST_STARTED: Notify = Notify::const_new();
fn first_run() {
    FIRST_RUNS.fetch_add(1, Ordering::SeqCst);
    FIRST_STARTED.notify_one();
}
scafra_scheduler::register_scheduled_task!("first_run", 60_000, first_run);

static SLOW_RUNS: AtomicUsize = AtomicUsize::new(0);
static SLOW_STARTED: Notify = Notify::const_new();
static SLOW_RETURNED: Notify = Notify::const_new();
static SLOW_RECEIVER: Mutex<Option<mpsc::Receiver<()>>> = Mutex::new(None);
fn slow_run() {
    let run = SLOW_RUNS.fetch_add(1, Ordering::SeqCst);
    SLOW_STARTED.notify_one();
    if run < 2 {
        if let Some(receiver) = SLOW_RECEIVER.lock().unwrap().as_ref() {
            let _ = receiver.recv();
        }
    }
    SLOW_RETURNED.notify_one();
}
scafra_scheduler::register_scheduled_task!("slow_run", 60_000, slow_run);

static PANIC_RUNS: AtomicUsize = AtomicUsize::new(0);
static PANIC_STARTED: Notify = Notify::const_new();
fn panic_run() {
    PANIC_RUNS.fetch_add(1, Ordering::SeqCst);
    PANIC_STARTED.notify_one();
    panic!("panic-payload-secret");
}
scafra_scheduler::register_scheduled_task!("panic_run", 60_000, panic_run);

static HEALTHY_RUNS: AtomicUsize = AtomicUsize::new(0);
static HEALTHY_STARTED: Notify = Notify::const_new();
fn healthy_run() {
    HEALTHY_RUNS.fetch_add(1, Ordering::SeqCst);
    HEALTHY_STARTED.notify_one();
}
scafra_scheduler::register_scheduled_task!("healthy_run", 60_000, healthy_run);

static INDEPENDENT_RUNS: AtomicUsize = AtomicUsize::new(0);
static INDEPENDENT_STARTED: Notify = Notify::const_new();
fn independent_run() {
    INDEPENDENT_RUNS.fetch_add(1, Ordering::SeqCst);
    INDEPENDENT_STARTED.notify_one();
}
scafra_scheduler::register_scheduled_task!("independent_run", 60_000, independent_run);

static CANCELLED_RUNS: AtomicUsize = AtomicUsize::new(0);
fn cancelled_run() {
    CANCELLED_RUNS.fetch_add(1, Ordering::SeqCst);
}
scafra_scheduler::register_scheduled_task!("cancelled_run", 60_000, cancelled_run);

fn config_for(enabled_task: &str, interval_ms: u64) -> SchedulerConfig {
    let names = [
        "first_run",
        "slow_run",
        "panic_run",
        "healthy_run",
        "independent_run",
        "cancelled_run",
    ];
    let tasks = names
        .into_iter()
        .map(|name| {
            (
                name.to_owned(),
                ScheduledTaskConfig {
                    enabled: name == enabled_task,
                    interval_ms: (name == enabled_task).then_some(interval_ms),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    SchedulerConfig {
        enabled: true,
        tasks,
    }
}

async fn wait_for_count(counter: &AtomicUsize, expected: usize) {
    while counter.load(Ordering::SeqCst) < expected {
        tokio::task::yield_now().await;
    }
}

#[test]
fn enabled_startup_requires_a_tokio_runtime() {
    let error = Scheduler::try_start(&config_for("first_run", 10))
        .expect_err("enabled registrations need a current runtime");
    assert!(matches!(error, SchedulerError::RuntimeUnavailable));
}

#[tokio::test(start_paused = true)]
async fn execution_cancellation_failure_and_instance_contracts_are_deterministic() {
    let logs = LogBuffer(Arc::new(Mutex::new(Vec::new())));
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(logs.clone())
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .expect("execution test installs its local process subscriber");

    // A configured interval overrides the 60-second registration default.
    FIRST_RUNS.store(0, Ordering::SeqCst);
    let first = Scheduler::try_start(&config_for("first_run", 10)).unwrap();
    let first_metrics = first.metrics();
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_millis(9)).await;
    assert_eq!(FIRST_RUNS.load(Ordering::SeqCst), 0);
    tokio::time::advance(std::time::Duration::from_millis(1)).await;
    FIRST_STARTED.notified().await;
    assert_eq!(FIRST_RUNS.load(Ordering::SeqCst), 1);
    first.shutdown().await;
    assert_eq!(first_metrics.snapshot().runs_finished, 1);

    // A disabled registration does not start. stop() cancels a sleeping loop
    // but returns without joining it.
    CANCELLED_RUNS.store(0, Ordering::SeqCst);
    let stopped = Scheduler::try_start(&config_for("cancelled_run", 10)).unwrap();
    stopped.stop();
    tokio::time::advance(std::time::Duration::from_millis(100)).await;
    assert_eq!(CANCELLED_RUNS.load(Ordering::SeqCst), 0);
    let dropped = Scheduler::try_start(&config_for("cancelled_run", 10)).unwrap();
    drop(dropped);
    tokio::time::advance(std::time::Duration::from_millis(100)).await;
    assert_eq!(CANCELLED_RUNS.load(Ordering::SeqCst), 0);
    let sleeping = Scheduler::try_start(&config_for("cancelled_run", 10)).unwrap();
    let shutdown_handle = sleeping.shutdown_handle();
    shutdown_handle.request_shutdown();
    tokio::task::yield_now().await;
    sleeping.shutdown().await;
    tokio::time::advance(std::time::Duration::from_millis(100)).await;
    assert_eq!(CANCELLED_RUNS.load(Ordering::SeqCst), 0);
    let disabled = Scheduler::try_start(&config_for("", 10)).unwrap();
    assert_eq!(disabled.metrics().snapshot().runs_started, 0);
    disabled.shutdown().await;

    // A slow callback occupies one blocking worker, never overlaps itself,
    // and does not queue missed intervals. Async shutdown waits for an active
    // callback to return.
    SLOW_RUNS.store(0, Ordering::SeqCst);
    let (release, receiver) = mpsc::channel();
    *SLOW_RECEIVER.lock().unwrap() = Some(receiver);
    let slow = Scheduler::try_start(&config_for("slow_run", 10)).unwrap();
    let slow_metrics = slow.metrics();
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_millis(10)).await;
    SLOW_STARTED.notified().await;
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    assert_eq!(SLOW_RUNS.load(Ordering::SeqCst), 1);
    assert_eq!(slow_metrics.snapshot().runs_started, 1);
    assert_eq!(slow_metrics.snapshot().active_callbacks, 1);

    release.send(()).unwrap();
    SLOW_RETURNED.notified().await;
    while slow_metrics.snapshot().runs_finished < 1 {
        tokio::task::yield_now().await;
    }
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_millis(9)).await;
    assert_eq!(SLOW_RUNS.load(Ordering::SeqCst), 1);
    tokio::time::advance(std::time::Duration::from_millis(1)).await;
    SLOW_STARTED.notified().await;
    assert_eq!(SLOW_RUNS.load(Ordering::SeqCst), 2);

    let shutdown = tokio::spawn(slow.shutdown());
    tokio::task::yield_now().await;
    assert!(
        !shutdown.is_finished(),
        "shutdown waits for an active callback"
    );
    release.send(()).unwrap();
    SLOW_RETURNED.notified().await;
    shutdown.await.unwrap();
    assert_eq!(slow_metrics.snapshot().runs_started, 2);
    assert_eq!(slow_metrics.snapshot().runs_finished, 2);
    assert_eq!(slow_metrics.snapshot().active_callbacks, 0);
    assert_eq!(SLOW_RUNS.load(Ordering::SeqCst), 2);

    // One callback panic is isolated; a separate registration still runs.
    PANIC_RUNS.store(0, Ordering::SeqCst);
    HEALTHY_RUNS.store(0, Ordering::SeqCst);
    let mut failure_config = config_for("panic_run", 10);
    failure_config.tasks.get_mut("healthy_run").unwrap().enabled = true;
    failure_config
        .tasks
        .get_mut("healthy_run")
        .unwrap()
        .interval_ms = Some(10);
    let failing = Scheduler::try_start(&failure_config).unwrap();
    let failing_metrics = failing.metrics();
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_millis(10)).await;
    PANIC_STARTED.notified().await;
    HEALTHY_STARTED.notified().await;
    wait_for_count(&PANIC_RUNS, 1).await;
    wait_for_count(&HEALTHY_RUNS, 1).await;
    while failing_metrics.snapshot().callback_panics == 0 {
        tokio::task::yield_now().await;
    }
    assert_eq!(failing_metrics.snapshot().callback_panics, 1);
    assert_eq!(HEALTHY_RUNS.load(Ordering::SeqCst), 1);
    let scheduler_logs = String::from_utf8(logs.0.lock().unwrap().clone()).unwrap();
    assert!(scheduler_logs.contains("outcome=\"panicked\""));
    assert!(!scheduler_logs.contains("panic-payload-secret"));
    failing.shutdown().await;

    // Two instances share registrations but not cancellation or metrics.
    INDEPENDENT_RUNS.store(0, Ordering::SeqCst);
    let first_instance = Scheduler::try_start(&config_for("independent_run", 10)).unwrap();
    let second_instance = Scheduler::try_start(&config_for("independent_run", 10)).unwrap();
    let first_instance_metrics = first_instance.metrics();
    let second_instance_metrics = second_instance.metrics();
    tokio::task::yield_now().await;
    tokio::time::advance(std::time::Duration::from_millis(10)).await;
    wait_for_count(&INDEPENDENT_RUNS, 2).await;
    while first_instance_metrics.snapshot().runs_finished < 1
        || second_instance_metrics.snapshot().runs_finished < 1
    {
        tokio::task::yield_now().await;
    }
    drop(first_instance);
    tokio::time::advance(std::time::Duration::from_millis(10)).await;
    wait_for_count(&INDEPENDENT_RUNS, 3).await;
    assert_eq!(first_instance_metrics.snapshot().runs_started, 1);
    assert_eq!(second_instance_metrics.snapshot().runs_started, 2);
    second_instance.shutdown().await;
}
