use std::collections::BTreeMap;

use scafra_scheduler::{
    ScheduledTaskConfig, ScheduledTaskRegistration, Scheduler, SchedulerConfig,
    SchedulerConfigError,
};

fn config_fixture() {}
scafra_scheduler::register_scheduled_task!("config_fixture", 100, config_fixture);

#[test]
fn scheduler_and_task_defaults_are_stable() {
    let config = SchedulerConfig::default();
    assert!(!config.enabled);
    assert!(config.tasks.is_empty());
    config.validate().expect("the default config is valid");

    let task = ScheduledTaskConfig::default();
    assert!(task.enabled);
    assert_eq!(task.interval_ms, None);
}

#[test]
fn legacy_start_and_stop_signatures_remain_available() {
    let scheduler = Scheduler::start(&SchedulerConfig::default());
    scheduler.stop();
}

#[test]
fn enabled_and_disabled_task_config_deserializes_with_registration_default() {
    let enabled: ScheduledTaskConfig = serde_yaml::from_str("{}").unwrap();
    assert!(enabled.enabled);
    assert_eq!(enabled.interval_ms, None);

    let disabled: ScheduledTaskConfig = serde_yaml::from_str("enabled: false\n").unwrap();
    assert!(!disabled.enabled);
    assert_eq!(disabled.interval_ms, None);

    let registration = scafra_scheduler::inventory::iter::<ScheduledTaskRegistration>
        .into_iter()
        .find(|registration| registration.name == "config_fixture")
        .expect("fixture registration should be linked");
    assert_eq!(registration.default_interval_ms, 100);
}

#[test]
fn missing_interval_uses_registration_default_and_override_is_preserved() {
    let mut tasks = BTreeMap::new();
    tasks.insert("config_fixture".to_owned(), ScheduledTaskConfig::default());
    let config = SchedulerConfig {
        enabled: true,
        tasks,
    };
    config
        .validate()
        .expect("registration default should be valid");

    let serialized = serde_yaml::to_string(&config).unwrap();
    let decoded: SchedulerConfig = serde_yaml::from_str(&serialized).unwrap();
    assert_eq!(decoded.tasks["config_fixture"].interval_ms, None);

    let override_config: SchedulerConfig = serde_yaml::from_str(
        "enabled: true\ntasks:\n  config_fixture:\n    enabled: true\n    interval_ms: 250\n",
    )
    .unwrap();
    override_config
        .validate()
        .expect("valid configured interval should override the default");
    assert_eq!(
        override_config.tasks["config_fixture"].interval_ms,
        Some(250)
    );
}

#[test]
fn zero_and_excessive_intervals_are_rejected_even_for_disabled_tasks() {
    let mut tasks = BTreeMap::new();
    tasks.insert(
        "config_fixture".to_owned(),
        ScheduledTaskConfig {
            enabled: false,
            interval_ms: Some(0),
        },
    );
    let config = SchedulerConfig {
        enabled: false,
        tasks,
    };
    assert_eq!(config.validate(), Err(SchedulerConfigError::ZeroInterval));

    let mut tasks = BTreeMap::new();
    tasks.insert(
        "config_fixture".to_owned(),
        ScheduledTaskConfig {
            enabled: true,
            interval_ms: Some(31_536_000_001),
        },
    );
    let config = SchedulerConfig {
        enabled: true,
        tasks,
    };
    assert_eq!(
        config.validate(),
        Err(SchedulerConfigError::IntervalTooLarge)
    );
}

#[test]
fn one_year_interval_is_a_large_valid_interval() {
    let mut tasks = BTreeMap::new();
    tasks.insert(
        "config_fixture".to_owned(),
        ScheduledTaskConfig {
            enabled: true,
            interval_ms: Some(31_536_000_000),
        },
    );
    SchedulerConfig {
        enabled: true,
        tasks,
    }
    .validate()
    .expect("one year is the supported maximum");
}

#[test]
fn unknown_names_are_rejected_without_echoing_configured_names() {
    let mut tasks = BTreeMap::new();
    tasks.insert(
        "credential-secret-that-must-not-leak".to_owned(),
        ScheduledTaskConfig::default(),
    );
    let config = SchedulerConfig {
        enabled: true,
        tasks,
    };
    let error = config.validate().expect_err("unknown tasks are invalid");
    assert_eq!(error, SchedulerConfigError::UnknownTask);
    assert!(!error.to_string().contains("credential-secret"));
    assert!(!format!("{error:?}").contains("credential-secret"));
}

#[test]
fn serde_round_trip_keeps_existing_field_names_and_defaults() {
    let config: SchedulerConfig = serde_yaml::from_str(
        "enabled: true\ntasks:\n  config_fixture:\n    enabled: false\n    interval_ms: 500\n",
    )
    .unwrap();
    let value = serde_yaml::to_value(&config).unwrap();
    assert_eq!(value["enabled"], true);
    assert_eq!(value["tasks"]["config_fixture"]["enabled"], false);
    assert_eq!(value["tasks"]["config_fixture"]["interval_ms"], 500);
}
