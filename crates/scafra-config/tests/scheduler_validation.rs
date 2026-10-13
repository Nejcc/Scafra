use scafra_config::{Config, ScafraConfig};
use scafra_scheduler::ScheduledTaskConfig;

fn valid_task() {}
scafra_scheduler::register_scheduled_task!("valid_task", 100, valid_task);

#[test]
fn application_validation_rejects_bad_scheduler_intervals_safely() {
    let mut config = ScafraConfig::default();
    config.scheduler.enabled = true;
    config.scheduler.tasks.insert(
        "valid_task".to_owned(),
        ScheduledTaskConfig {
            enabled: true,
            interval_ms: Some(0),
        },
    );

    let error = config
        .validate()
        .expect_err("zero interval must fail validation");
    let details = ScafraConfig::validation_details(&error).unwrap();
    assert_eq!(details.field, "scheduler.tasks.interval_ms");
    assert_eq!(details.message, "must be greater than zero milliseconds");
    assert!(!error.to_string().contains("0"));
}

#[test]
fn unknown_scheduler_names_are_rejected_without_echoing_them() {
    let mut config = ScafraConfig::default();
    config.scheduler.tasks.insert(
        "secret-task-name-that-must-not-leak".to_owned(),
        ScheduledTaskConfig::default(),
    );

    let error = config
        .validate()
        .expect_err("unknown task name must fail validation");
    assert!(!error.to_string().contains("secret-task-name"));
    assert!(!format!("{error:?}").contains("secret-task-name"));
}

#[test]
fn default_application_config_keeps_scheduler_opt_in() {
    let config = ScafraConfig::default();
    config
        .validate()
        .expect("valid registered defaults should validate");
    assert!(!config.scheduler.enabled);
    assert!(config.scheduler.tasks.is_empty());
}
