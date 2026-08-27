pub mod models;
mod queries;

pub use queries::{latest_thread, save_thread};

pub fn new_thread_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
