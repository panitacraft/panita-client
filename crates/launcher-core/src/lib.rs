//! UI-independent domain logic for Panita Client.

pub mod accounts;
pub mod catalog;
pub mod error;
pub mod logging;
pub mod operation;
pub mod paths;
pub mod settings;
pub mod storage;

pub const APPLICATION_NAME: &str = "Panita Client";

pub fn application_name() -> &'static str {
    APPLICATION_NAME
}
