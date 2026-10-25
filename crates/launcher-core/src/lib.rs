//! UI-independent domain logic for Panita Client.

pub mod accounts;
pub mod authentication;
pub mod catalog;
pub mod error;
mod instance_layout;
pub mod logging;
pub mod minecraft;
pub mod oauth;
pub mod operation;
pub mod paths;
pub mod registry;
pub mod sessions;
pub mod settings;
pub mod storage;
mod storage_json;

pub const APPLICATION_NAME: &str = "Panita Client";

pub fn application_name() -> &'static str {
    APPLICATION_NAME
}
