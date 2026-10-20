use std::sync::Arc;

use scafra::service;

use crate::beans::Worker;

#[service]
pub struct WorkerService {
    worker: Arc<Worker>,
}

impl WorkerService {
    pub fn is_running(&self) -> bool {
        let _ = &self.worker;
        true
    }
}
