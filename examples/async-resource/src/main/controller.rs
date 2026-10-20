use scafra::{controller, routes};
#[allow(unused_imports)]
use scafra::get;

use crate::service::WorkerService;

#[controller("/lifecycle")]
pub struct LifecycleController {
    service: WorkerService,
}

#[routes]
impl LifecycleController {
    #[get("/ready")]
    async fn ready(&self) -> &'static str {
        if self.service.is_running() {
            "ready"
        } else {
            "stopping"
        }
    }
}
