use scafra_scheduler::{SchedulerConfig, SchedulerConfigError};

fn first_task() {}
fn second_task() {}
scafra_scheduler::register_scheduled_task!("duplicate", 1, first_task);
scafra_scheduler::register_scheduled_task!("duplicate", 2, second_task);

#[test]
fn duplicate_registration_names_fail_validation() {
    assert_eq!(
        SchedulerConfig::default().validate(),
        Err(SchedulerConfigError::DuplicateRegistration)
    );
}
