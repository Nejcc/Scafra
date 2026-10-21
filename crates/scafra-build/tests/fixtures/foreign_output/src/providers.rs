use scafra::bean;
use std::time::Duration;

#[bean]
pub fn resource() -> Duration {
    Duration::ZERO
}
