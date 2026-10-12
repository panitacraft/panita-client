//! UI-independent domain logic for Panita Client.

pub mod error;
pub mod operation;
pub mod paths;
pub mod settings;
pub mod storage;

pub const APPLICATION_NAME: &str = "Panita Client";

pub fn application_name() -> &'static str {
    APPLICATION_NAME
}
