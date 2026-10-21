#[scafra::main]
async fn main() {}

#[cfg(test)]
mod tests {
    use super::__scafra_graph;
    use std::sync::Mutex;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn typed_providers_construct_non_default_dependencies_once_in_graph_order() {
        let _test_lock = TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        crate::providers::reset_events();

        let _graph = __scafra_graph::compose().expect("the synchronous graph should compose");

        assert_eq!(
            crate::providers::events(),
            ["credentials", "runtime_config", "external_client"]
        );
    }

    #[test]
    fn generated_application_router_rejects_unknown_health_checks_at_startup() {
        let _test_lock = TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let graph = __scafra_graph::compose().expect("the graph should compose");
        let actuator = scafra::ActuatorConfig {
            health: scafra::HealthConfig {
                checks: vec!["misspelled-dependency".to_owned()],
                ..scafra::HealthConfig::default()
            },
            ..scafra::ActuatorConfig::all()
        };

        let (router, _resources) = graph.build_router_and_resources(
            &actuator,
            &scafra::SecurityConfig::default(),
            None,
        );
        let error = router.expect_err("invalid readiness configuration must stop router startup");
        assert!(error.to_string().contains("unknown check name"));
    }
}
