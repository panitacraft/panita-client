//! First-install policy for the `defaults` and `enforced` configuration archives and
//! for mrpack overrides. Nothing here overwrites: a path that already exists is kept
//! whole and reported as skipped, and a default is applied only to a path that was
//! never offered before.
use std::collections::BTreeSet;
use std::io::{Read, Seek};

use super::{
    archive::{ArchiveEntry, Opened},
    gamefs::{Entry, GameDir},
    invalid, path_key,
    state::{ManagedFile, Ownership, SkippedConfiguration},
};
use crate::{
    error::{CoreError, ErrorCode},
    minecraft::download::Cancellation,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Defaults,
    Enforced,
    /// An mrpack override layer: pack content under the content folders, and
    /// configuration everywhere else.
    Override,
}

/// What applying one archive changed and what it left alone.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub files: Vec<ManagedFile>,
    /// Every default offered, applied or not, so a deleted default never returns.
    pub consumed_defaults: BTreeSet<String>,
    /// Existing paths kept whole instead of being replaced.
    pub skipped: Vec<SkippedConfiguration>,
    /// Paths this call created; the install journal uses them to roll back.
    pub written: Vec<String>,
}

/// A path may not be offered by both archives.
pub fn ensure_disjoint(first: &[ArchiveEntry], second: &[ArchiveEntry]) -> Result<(), CoreError> {
    let names: BTreeSet<_> = first.iter().map(|e| path_key(&e.path)).collect();
    if second
        .iter()
        .any(|entry| names.contains(&path_key(&entry.path)))
    {
        return Err(invalid());
    }
    Ok(())
}

pub fn apply<R: Read + Seek>(
    opened: &mut Opened<R>,
    kind: Kind,
    game: &GameDir,
    already_consumed: &BTreeSet<String>,
    budget: &mut u64,
    cancel: &Cancellation,
) -> Result<Outcome, CoreError> {
    let consumed: BTreeSet<_> = already_consumed.iter().map(|path| path_key(path)).collect();
    let mut outcome = Outcome::default();
    for entry in opened.entries.clone() {
        cancel.check()?;
        if kind == Kind::Defaults {
            if consumed.contains(&path_key(&entry.path)) {
                continue;
            }
            outcome.consumed_defaults.insert(entry.path.clone());
        }
        let content = kind == Kind::Override && super::mrpack::is_content(&entry.path);
        let ownership = match kind {
            Kind::Defaults => Ownership::Defaults,
            Kind::Enforced => Ownership::Enforced,
            Kind::Override if content => Ownership::Content,
            Kind::Override => {
                // A deleted override is never recreated, as for deleted defaults.
                outcome.consumed_defaults.insert(entry.path.clone());
                Ownership::Override
            }
        };
        let absent = game.entry(&entry.path)? == Entry::Absent;
        if kind == Kind::Override && !content && absent && consumed.contains(&path_key(&entry.path))
        {
            // Offered before and deleted since: the player's choice stands.
            continue;
        }
        if content && !absent {
            // Immutable pack content never adopts a file the player already placed.
            return Err(CoreError::new(ErrorCode::PackConflict));
        }
        let digest = if absent {
            let mut digest = None;
            game.write_new(&entry.path, |output| {
                digest = Some(opened.stream(&entry, budget, cancel, Some(output))?);
                Ok(())
            })?;
            outcome.written.push(entry.path.clone());
            digest.ok_or_else(invalid)?
        } else {
            opened.stream(&entry, budget, cancel, None)?
        };
        outcome.files.push(ManagedFile {
            path: entry.path.clone(),
            ownership,
            desired: digest.hashes.clone(),
            size: digest.size,
            baseline: absent.then(|| digest.hashes.clone()),
        });
        if !absent {
            outcome.skipped.push(SkippedConfiguration {
                path: entry.path,
                desired: digest.hashes,
            });
        }
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests;
