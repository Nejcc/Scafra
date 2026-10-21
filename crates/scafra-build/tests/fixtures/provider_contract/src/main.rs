#[scafra::main]
async fn main() {}

#[cfg(test)]
mod tests {
    use super::__scafra_graph;

    #[test]
    fn typed_providers_construct_non_default_dependencies_once_in_graph_order() {
        crate::providers::reset_events();

        let _graph = __scafra_graph::compose().expect("the synchronous graph should compose");

        assert_eq!(
            crate::providers::events(),
            ["credentials", "runtime_config", "external_client"]
        );
    }
}
