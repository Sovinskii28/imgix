use std::sync::Arc;
use tokio::sync::Semaphore;

pub const MAX_CONCURRENT_COMPRESSIONS: usize = 4;

#[derive(Clone)]
pub struct AppState {
    pub compression_permits: Arc<Semaphore>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            compression_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_COMPRESSIONS)),
        }
    }
}
