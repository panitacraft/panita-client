use serde::Serialize;
use uuid::Uuid;

/// Public catalog metadata only. Installation and account state never travel in
/// this facade; Phase 3 will populate it from validated API records.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogInstance {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub minecraft_version: String,
    pub enabled: bool,
}

pub fn catalog_snapshot() -> Vec<CatalogInstance> {
    Vec::new()
}
