use scafra_scheduler::{SchedulerConfig, SchedulerConfigError};

fn invalid_name_task() {}
scafra_scheduler::register_scheduled_task!(
    "aaa_this_registration_name_is_longer_than_sixty_four_bytes_and_is_invalid",
    1,
    invalid_name_task
);

fn first_duplicate_task() {}
fn second_duplicate_task() {}
scafra_scheduler::register_scheduled_task!("duplicate", 1, first_duplicate_task);
scafra_scheduler::register_scheduled_task!("duplicate", 2, second_duplicate_task);

#[test]
fn registration_names_are_bounded() {
    let error = SchedulerConfig::default()
        .validate()
        .expect_err("invalid linked registration names fail validation");
    assert_eq!(error, SchedulerConfigError::InvalidRegistration);
    assert!(!error.to_string().contains("sixty_four"));
}
