#[scafra::main]
async fn main() {}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use scafra::{
        core::{AsyncResourceSet, GraphError},
        foundation::{ShutdownPolicy, ShutdownReason},
        ActuatorConfig, SecurityConfig, StartupError,
    };
    use tokio::sync::MutexGuard;

    use super::{beans, __scafra_graph};

    static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    async fn serial() -> MutexGuard<'static, ()> {
        SERIAL.lock().await
    }

    fn reset() {
        beans::reset_for_test();
    }

    async fn compose_resources() -> (scafra::web::axum::Router, __scafra_graph::Resources) {
        let graph = __scafra_graph::compose_async()
            .await
            .expect("all test providers should initialize");
        let (router, resources) = graph.build_router_and_resources(
            &ActuatorConfig::default(),
            &SecurityConfig::default(),
            None,
        );
        (router.expect("example routes should build"), resources)
    }

    #[tokio::test]
    async fn initializes_once_in_dependency_order_and_drains_owned_task() {
        let _serial = serial().await;
        reset();

        let (router, resources) = compose_resources().await;
        beans::WORKER_STARTED.notified().await;
        assert_eq!(
            beans::events(),
            vec!["database.initialize", "client.initialize", "worker.initialize", "worker.started"]
        );
        drop(router);

        resources
            .shutdown(
                ShutdownReason::ApplicationRequest,
                ShutdownPolicy::new(Duration::from_secs(2), true),
            )
            .await
            .expect("cooperative resources should shut down cleanly");
        assert!(!beans::DATABASE_OPEN.load(std::sync::atomic::Ordering::SeqCst));

        assert_eq!(
            beans::events(),
            vec![
                "database.initialize",
                "client.initialize",
                "worker.initialize",
                "worker.started",
                "worker.shutdown",
                "worker.cancelled",
                "worker.finished",
                "worker.task_dropped",
                "client.shutdown",
                "database.shutdown",
            ]
        );
    }

    #[tokio::test]
    async fn startup_failure_rolls_back_in_reverse_order_and_keeps_both_errors() {
        let _serial = serial().await;
        reset();
        beans::FAIL_WORKER.store(true, std::sync::atomic::Ordering::SeqCst);
        beans::FAIL_CLIENT_SHUTDOWN.store(true, std::sync::atomic::Ordering::SeqCst);

        let error = match __scafra_graph::compose_async().await {
            Ok(_) => panic!("worker initialization is configured to fail"),
            Err(error) => error,
        };
        let original = error
            .original_provider_failure()
            .expect("startup error should retain its original provider failure");
        assert_eq!(original.provider(), "crate::beans::worker");
        assert!(std::error::Error::source(&original)
            .and_then(|source| source.downcast_ref::<beans::ResourceError>())
            .is_some());
        assert_eq!(error.cleanup_failures().len(), 1);
        assert_eq!(
            error.cleanup_failures()[0].provider(),
            "crate::beans::service_client"
        );
        assert_eq!(
            beans::events(),
            vec![
                "database.initialize",
                "client.initialize",
                "worker.initialize",
                "client.shutdown",
                "database.shutdown",
            ]
        );
    }

    #[tokio::test]
    async fn shutdown_continues_after_a_resource_cleanup_failure() {
        let _serial = serial().await;
        reset();
        beans::FAIL_CLIENT_SHUTDOWN.store(true, std::sync::atomic::Ordering::SeqCst);
        let (router, resources) = compose_resources().await;
        beans::WORKER_STARTED.notified().await;
        drop(router);

        let error = resources
            .shutdown(
                ShutdownReason::ApplicationRequest,
                ShutdownPolicy::default(),
            )
            .await
            .expect_err("client cleanup is configured to fail");
        assert_eq!(error.cleanup_failures().len(), 1);
        assert_eq!(
            error.cleanup_failures()[0].provider(),
            "crate::beans::service_client"
        );
        assert!(beans::events().contains(&"database.shutdown"));
        assert!(!beans::DATABASE_OPEN.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn resource_shutdown_waits_for_cooperative_task_join() {
        let _serial = serial().await;
        reset();
        let (router, resources) = compose_resources().await;
        beans::WORKER_STARTED.notified().await;
        drop(router);

        resources
            .shutdown(ShutdownReason::ApplicationRequest, ShutdownPolicy::default())
            .await
            .expect("cooperative task should join");
        let events = beans::events();
        let finished = events
            .iter()
            .position(|event| *event == "worker.finished")
            .expect("task should report completion");
        let client_shutdown = events
            .iter()
            .position(|event| *event == "client.shutdown")
            .expect("dependent resource cleanup should follow task join");
        assert!(finished < client_shutdown);
    }

    #[tokio::test]
    async fn failed_required_provider_prevents_application_startup() {
        let _serial = serial().await;
        reset();
        beans::FAIL_WORKER.store(true, std::sync::atomic::Ordering::SeqCst);

        let result = scafra::run_with_resources(|config| async move {
            let graph = __scafra_graph::compose_async().await?;
            let (router, resources) = graph.build_router_and_resources(
                &config.actuator,
                &config.security,
                config
                    .server
                    .request_timeout_seconds
                    .map(Duration::from_secs),
            );
            router
                .map(|router| (router, resources))
                .map_err(StartupError::from)
        })
        .await;

        assert!(matches!(result, Err(StartupError::Graph(_))));
        assert!(beans::events().contains(&"database.shutdown"));
        assert!(!beans::events().contains(&"worker.started"));
    }

    #[tokio::test]
    async fn task_that_ignores_cancellation_is_aborted_at_its_declared_grace_period() {
        let _serial = serial().await;
        reset();
        beans::IGNORE_WORKER_CANCELLATION.store(true, std::sync::atomic::Ordering::SeqCst);
        let (router, resources) = compose_resources().await;
        beans::WORKER_STARTED.notified().await;
        drop(router);

        tokio::time::pause();
        let policy = ShutdownPolicy::new(Duration::from_secs(3), true);
        let shutdown = resources.shutdown(ShutdownReason::Signal, policy);
        tokio::pin!(shutdown);
        tokio::select! {
            biased;
            result = &mut shutdown => panic!("uncooperative task should exceed its grace period: {result:?}"),
            _ = tokio::task::yield_now() => {},
        }
        tokio::time::advance(policy.grace_period()).await;
        let error = shutdown.await.expect_err("the task exceeded its grace period");
        assert!(matches!(error, GraphError::ProviderFailure(_)));
        assert_eq!(error.cleanup_failures().len(), 1);
        assert!(beans::events().contains(&"worker.task_dropped"));
        assert!(beans::events().contains(&"database.shutdown"));
    }
}
