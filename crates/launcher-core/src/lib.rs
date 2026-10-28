//! UI-independent domain logic for Panita Client.

pub mod accounts;
pub mod activation;
pub mod authentication;
pub mod catalog;
#[cfg(any(windows, test))]
pub mod data_removal;
mod device;
pub mod error;
mod instance_layout;
pub mod loader;
pub mod logging;
pub mod minecraft;
pub mod modpack;
pub mod neoforge;
pub mod oauth;
pub mod operation;
pub mod panita_api;
pub mod panita_session;
pub mod paths;
pub mod registry;
pub mod sessions;
pub mod settings;
pub mod storage;
mod storage_json;
#[cfg(test)]
pub(crate) mod test_http;

pub const APPLICATION_NAME: &str = "Panita Client";

pub fn application_name() -> &'static str {
    APPLICATION_NAME
}
