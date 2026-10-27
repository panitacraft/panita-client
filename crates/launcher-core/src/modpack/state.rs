//! Versioned pack state to embed in a future installed-plan schema. No disk writer
//! is enabled here; vanilla schema 4 and its recovery readers remain authoritative.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PackIdentity {
    Custom {
        version_id: Uuid,
        published_at: String,
        label: Option<String>,
    },
    Modrinth {
        project_id: String,
        version_id: String,
        archive_sha512: String,
        index_sha256: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ownership {
    Content,
    Defaults,
    Enforced,
    Override,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedFile {
    pub path: String,
    pub ownership: Ownership,
    /// Desired content in the exact pack, independent of preserved local edits.
    pub desired: Hashes,
    pub size: u64,
    /// Hash at last successful publication; never updated merely by observing edits.
    pub baseline: Option<Hashes>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkippedConfiguration {
    pub path: String,
    pub desired: Hashes,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackState {
    pub schema: u32,
    pub id: Uuid,
    pub source: PackIdentity,
    pub minecraft_version: String,
    pub loader: String,
    pub loader_version: String,
    pub files: Vec<ManagedFile>,
    /// Includes defaults the player subsequently deleted; never inferred from existence.
    pub consumed_defaults: BTreeSet<String>,
    pub skipped_configuration: Vec<SkippedConfiguration>,
}

impl PackState {
    pub fn decode(bytes: &[u8], id: Uuid) -> Result<Self, CoreError> {
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(invalid());
        }
        let state: Self = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        state.validate(id)?;
        Ok(state)
    }

    pub fn validate(&self, id: Uuid) -> Result<(), CoreError> {
        if self.schema != 1
            || self.id != id
            || id.is_nil()
            || !pin(&self.minecraft_version)
            || self.minecraft_version.len() > 32
            || !pin(&self.loader)
            || !pin(&self.loader_version)
            || self.files.len() > MAX_FILES
            || self.consumed_defaults.len() > MAX_FILES
            || self.skipped_configuration.len() > MAX_FILES
        {
            return Err(invalid());
        }
        match &self.source {
            PackIdentity::Custom {
                version_id,
                published_at,
                label,
            } => {
                if version_id.is_nil()
                    || !date(published_at)
                    || label.as_deref().is_some_and(|v| !text(v, 32))
                {
                    return Err(invalid());
                }
            }
            PackIdentity::Modrinth {
                project_id,
                version_id,
                archive_sha512,
                index_sha256,
            } => {
                if !pin(project_id)
                    || !pin(version_id)
                    || !hex(archive_sha512, 128)
                    || !hex(index_sha256, 64)
                {
                    return Err(invalid());
                }
            }
        }
        let mut paths = BTreeMap::new();
        let mut total = 0u64;
        for file in &self.files {
            game_path(&file.path)?;
            file.desired.validate()?;
            if let Some(baseline) = &file.baseline {
                baseline.validate()?;
            }
            if file.size > MAX_FILE_BYTES
                || paths.insert(file.path.to_ascii_lowercase(), file).is_some()
                || (file.ownership == Ownership::Content
                    && file.baseline.as_ref() != Some(&file.desired))
            {
                return Err(invalid());
            }
            total = total.checked_add(file.size).ok_or_else(invalid)?;
        }
        if total > MAX_PACK_BYTES {
            return Err(invalid());
        }
        for path in paths.keys() {
            for (end, _) in path.match_indices('/') {
                if paths.contains_key(&path[..end]) {
                    return Err(invalid());
                }
            }
        }
        let mut consumed = BTreeSet::new();
        for path in &self.consumed_defaults {
            game_path(path)?;
            if !consumed.insert(path.to_ascii_lowercase()) {
                return Err(invalid());
            }
        }
        let mut skipped = BTreeSet::new();
        for skip in &self.skipped_configuration {
            skip.desired.validate()?;
            let key = skip.path.to_ascii_lowercase();
            let file = paths.get(&key).ok_or_else(invalid)?;
            if skip.path != file.path
                || file.ownership == Ownership::Content
                || skip.desired != file.desired
                || !skipped.insert(key)
            {
                return Err(invalid());
            }
        }
        for file in &self.files {
            if file.ownership == Ownership::Defaults
                && !consumed.contains(&file.path.to_ascii_lowercase())
            {
                return Err(invalid());
            }
            if file.ownership != Ownership::Content
                && file.baseline.as_ref() != Some(&file.desired)
                && !skipped.contains(&file.path.to_ascii_lowercase())
            {
                return Err(invalid());
            }
        }
        Ok(())
    }
}
