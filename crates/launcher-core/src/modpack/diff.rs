//! Ownership-aware comparison of an installed pack with a new plan. It is pure: the
//! caller supplies what each path currently holds, and nothing here touches a disk.
//!
//! The rules follow one principle. Bytes the launcher can prove it wrote (they still
//! match their recorded baseline) may be replaced or removed. Anything else belongs to
//! the player: configuration is kept whole and reported, content that would have to be
//! replaced is a conflict the player must resolve, and nothing is ever deleted
//! because it differs from the pack.
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::{
    Hashes, path_key,
    state::{ManagedFile, Ownership, PackState, SkippedConfiguration},
};
use crate::error::CoreError;

/// One file the new plan wants, with the exact bytes it must have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Incoming {
    pub path: String,
    pub ownership: Ownership,
    pub desired: Hashes,
    pub size: u64,
    /// A file the player placed by hand (a manual download) counts as installed when
    /// its bytes match, instead of being a collision.
    pub adopt_equal: bool,
    /// Opaque to the diff: where the caller finds the new bytes.
    pub source: usize,
}

/// What a path currently holds, with the hashes of its bytes when it is a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Local {
    Absent,
    File(Hashes),
    /// A folder, link, hard-linked file or other node.
    Unsafe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// A managed content file the player changed would have to be replaced.
    Edited,
    /// A file the player already has sits where the pack wants to install one.
    Occupied,
    /// A folder or link sits there; it can be neither replaced nor moved aside.
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Conflict {
    pub path: String,
    pub reason: Reason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// The path is absent: create the new bytes.
    Write,
    /// The old bytes are the launcher's own: set them aside and write the new ones.
    Replace,
    /// The old bytes are the launcher's own and the pack dropped them: set them aside.
    Remove,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub path: String,
    pub op: Op,
    pub incoming: Option<Incoming>,
    /// The existing file is the player's: it moves to the pack backups first.
    pub backup: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diff {
    pub steps: Vec<Step>,
    pub conflicts: Vec<Conflict>,
    /// The managed list once every step has been applied.
    pub files: Vec<ManagedFile>,
    pub consumed: BTreeSet<String>,
    pub skipped: Vec<SkippedConfiguration>,
    /// Paths the pack no longer manages; the files stay as the player's.
    pub released: Vec<String>,
    /// Managed content the player changed that the pack does not touch.
    pub edited: Vec<String>,
}

impl Diff {
    /// Conflicts that moving a file aside cannot resolve.
    pub fn unresolvable(&self) -> bool {
        self.conflicts
            .iter()
            .any(|conflict| conflict.reason == Reason::Blocked)
    }

    pub fn changes_anything(&self) -> bool {
        !self.steps.is_empty()
    }
}

/// True when every algorithm both sides name agrees, and at least one is shared.
pub fn same(left: &Hashes, right: &Hashes) -> bool {
    let mut shared = false;
    for (a, b) in [(&left.sha1, &right.sha1), (&left.sha512, &right.sha512)] {
        if let (Some(a), Some(b)) = (a, b) {
            if a != b {
                return false;
            }
            shared = true;
        }
    }
    shared
}

fn is_content(ownership: &Ownership) -> bool {
    *ownership == Ownership::Content
}

/// Compare the installed state with `incoming`. \`local\` reports what a path holds now.
pub fn diff(
    old: &PackState,
    incoming: Vec<Incoming>,
    local: &mut dyn FnMut(&str) -> Result<Local, CoreError>,
) -> Result<Diff, CoreError> {
    let key = |path: &str| path_key(path);
    let old_files: BTreeMap<String, &ManagedFile> = old
        .files
        .iter()
        .map(|file| (key(&file.path), file))
        .collect();
    let mut consumed = old.consumed_defaults.clone();
    let consumed_keys: BTreeSet<String> = consumed.iter().map(|path| key(path)).collect();
    let mut out = Diff::default();
    let mut seen = BTreeSet::new();

    for new in incoming {
        let k = key(&new.path);
        seen.insert(k.clone());
        let previous = old_files.get(&k).copied();
        let here = local(&new.path)?;
        let entry = |baseline: Option<Hashes>| ManagedFile {
            path: new.path.clone(),
            ownership: new.ownership.clone(),
            desired: new.desired.clone(),
            size: new.size,
            baseline,
        };
        // Offered configuration is remembered so a deleted file is never recreated.
        let remember = new.ownership != Ownership::Enforced && !is_content(&new.ownership);
        if remember {
            consumed.insert(new.path.clone());
        }
        let step = |op: Op, backup: bool| Step {
            path: new.path.clone(),
            op,
            incoming: Some(new.clone()),
            backup,
        };
        match here {
            Local::Absent => {
                let player_deleted = !is_content(&new.ownership)
                    && new.ownership != Ownership::Enforced
                    && (previous.is_some() || consumed_keys.contains(&k));
                if player_deleted {
                    // The player removed it on purpose; the pack does not bring it back.
                    continue;
                }
                out.steps.push(step(Op::Write, false));
                out.files.push(entry(Some(new.desired.clone())));
            }
            Local::File(found) => {
                if new.ownership == Ownership::Defaults {
                    // A default seeds a path once. It never replaces a file that is there.
                    if let Some(previous) = previous {
                        out.files.push(previous.clone());
                        continue;
                    }
                    if consumed_keys.contains(&k) {
                        continue;
                    }
                }
                let baseline = previous.and_then(|file| file.baseline.as_ref());
                let ours = baseline.is_some_and(|baseline| same(&found, baseline));
                let equal = same(&found, &new.desired);
                if equal && (previous.is_some() || !is_content(&new.ownership) || new.adopt_equal) {
                    out.files.push(entry(Some(new.desired.clone())));
                } else if ours {
                    out.steps.push(step(Op::Replace, false));
                    out.files.push(entry(Some(new.desired.clone())));
                } else if !is_content(&new.ownership) {
                    // Edited, or a player file that was never ours: keep the whole file.
                    out.files.push(entry(baseline.cloned()));
                    out.skipped.push(SkippedConfiguration {
                        path: new.path.clone(),
                        desired: new.desired.clone(),
                    });
                } else if previous.is_some_and(|file| same(&file.desired, &new.desired)) {
                    // The pack did not change this file and the player did.
                    out.files
                        .push(previous.cloned().ok_or_else(super::invalid)?);
                    out.edited.push(new.path.clone());
                } else {
                    let reason = if previous.is_some() {
                        Reason::Edited
                    } else {
                        Reason::Occupied
                    };
                    out.conflicts.push(Conflict {
                        path: new.path.clone(),
                        reason,
                    });
                    out.steps.push(step(Op::Replace, true));
                    out.files.push(entry(Some(new.desired.clone())));
                }
            }
            Local::Unsafe => {
                if is_content(&new.ownership) {
                    out.conflicts.push(Conflict {
                        path: new.path.clone(),
                        reason: Reason::Blocked,
                    });
                } else {
                    out.files
                        .push(entry(previous.and_then(|file| file.baseline.clone())));
                    out.skipped.push(SkippedConfiguration {
                        path: new.path.clone(),
                        desired: new.desired.clone(),
                    });
                }
            }
        }
    }

    // What the previous pack managed and the new one no longer names.
    for file in &old.files {
        if seen.contains(&key(&file.path)) {
            continue;
        }
        if !is_content(&file.ownership) {
            // Configuration is never deleted; it simply stops being managed.
            out.released.push(file.path.clone());
            continue;
        }
        match local(&file.path)? {
            Local::Absent => {}
            Local::File(found)
                if file
                    .baseline
                    .as_ref()
                    .is_some_and(|baseline| same(&found, baseline)) =>
            {
                out.steps.push(Step {
                    path: file.path.clone(),
                    op: Op::Remove,
                    incoming: None,
                    backup: false,
                });
            }
            Local::File(_) | Local::Unsafe => out.released.push(file.path.clone()),
        }
    }
    out.consumed = consumed;
    Ok(out)
}

/// Repair of the installed exact pin. Only content is touched: a missing file is
/// restored and a changed one is set aside as the player's before the pack's bytes
/// return. Configuration, history and every recorded baseline stay as they are.
pub fn repair_diff(
    old: &PackState,
    incoming: Vec<Incoming>,
    local: &mut dyn FnMut(&str) -> Result<Local, CoreError>,
) -> Result<Diff, CoreError> {
    let mut out = Diff {
        files: old.files.clone(),
        consumed: old.consumed_defaults.clone(),
        skipped: old.skipped_configuration.clone(),
        ..Diff::default()
    };
    for new in incoming {
        // The pin is exact: every file must already be recorded with the same bytes.
        let recorded = old
            .files
            .iter()
            .find(|file| path_key(&file.path) == path_key(&new.path))
            .filter(|file| is_content(&file.ownership) && same(&file.desired, &new.desired))
            .ok_or_else(super::invalid)?;
        let step = |backup: bool| Step {
            path: recorded.path.clone(),
            op: if backup { Op::Replace } else { Op::Write },
            incoming: Some(new.clone()),
            backup,
        };
        match local(&recorded.path)? {
            Local::Absent => out.steps.push(step(false)),
            Local::File(found) if same(&found, &new.desired) => {}
            Local::File(_) => {
                out.edited.push(recorded.path.clone());
                out.steps.push(step(true));
            }
            Local::Unsafe => out.conflicts.push(Conflict {
                path: recorded.path.clone(),
                reason: Reason::Blocked,
            }),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
