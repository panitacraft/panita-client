//! Automatic RAM allocation (owner decision, 2026-11-24). A pure function from what
//! is being launched and the memory of the machine to the game's heap size; reading
//! the machine and building the command happen elsewhere, so every case is testable.
use serde::Serialize;

use crate::catalog::Variant;
use crate::memory::MemoryStatus;
use crate::modpack::state::{PackIdentity, PackState};

/// Allocations are multiples of this size.
const STEP_MIB: u64 = 256;
/// Hard upper bound: larger heaps lengthen garbage-collection pauses and Minecraft
/// does not benefit from them.
pub const CAP_MIB: u64 = 16_384;
/// Used when the installed memory cannot be read: today's global default.
const UNKNOWN_MACHINE_CAP_MIB: u64 = 4_096;
/// The operating system keeps at least this much, or a quarter of the memory.
const SYSTEM_RESERVE_MIB: u64 = 2_048;

/// What kind of game is launched, which decides how much memory it wants.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RamProfile {
    Vanilla,
    Lite,
    Standard,
    Deluxe,
}

impl RamProfile {
    /// Vanilla has no pack. A Modrinth pack has no variants and counts as Standard.
    pub fn of(pack: Option<&PackState>) -> Self {
        match pack.map(|pack| &pack.source) {
            None => Self::Vanilla,
            Some(PackIdentity::Modrinth { .. }) => Self::Standard,
            Some(PackIdentity::Custom { variant, .. }) => match variant {
                Variant::Lite => Self::Lite,
                Variant::Standard => Self::Standard,
                Variant::Deluxe => Self::Deluxe,
            },
        }
    }

    /// The heap the profile asks for when the machine can give it.
    pub const fn target_mib(self) -> u64 {
        match self {
            Self::Vanilla | Self::Lite => 4_096,
            Self::Standard => 6_144,
            Self::Deluxe => 8_192,
        }
    }

    /// The least that is launched even when the machine cannot provide it.
    pub const fn floor_mib(self) -> u64 {
        match self {
            Self::Vanilla => 2_048,
            _ => 3_072,
        }
    }
}

/// Why the value is what it is, for the support log and the settings preview.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RamBasis {
    /// The machine can give everything the profile asks for.
    Target,
    /// Less than the profile asks for, because the machine's memory cannot spare more.
    Limited,
    /// The profile's minimum; the machine's memory cannot really spare it.
    Floor,
    /// The installed memory could not be read.
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct RamResolution {
    pub mib: u32,
    pub basis: RamBasis,
}

impl RamResolution {
    /// A fixed note for the session log: no account, path or other private data.
    pub fn note(&self, profile: RamProfile, memory: &MemoryStatus) -> String {
        let total = memory
            .total_mib
            .map_or_else(|| "unknown".to_owned(), |value| value.to_string());
        format!(
            "ram auto profile={profile:?} heap_mib={} basis={:?} total_mib={total}",
            self.mib, self.basis,
        )
    }
}

/// The heap for an automatic allocation.
///
/// It depends on the memory installed in the machine, never on how much happens to be
/// free at the moment (owner clarification, 2026-11-25). The machine can spare its total
/// memory minus the system reserve (the larger of 2 GiB and a quarter of it). The
/// profile's target is lowered to that, rounded down to a multiple of 256 MiB and kept
/// within the profile's floor and the hard cap. Without a reading of the total memory
/// the target is held to 4 GiB.
pub fn resolve(profile: RamProfile, memory: &MemoryStatus) -> RamResolution {
    let target = profile.target_mib().min(CAP_MIB);
    let limit = memory.total_mib.map_or(UNKNOWN_MACHINE_CAP_MIB, |total| {
        total.saturating_sub((total / 4).max(SYSTEM_RESERVE_MIB))
    });
    let wanted = target.min(limit);
    let rounded = wanted / STEP_MIB * STEP_MIB;
    let floor = profile.floor_mib();
    let (mib, basis) = if rounded < floor {
        (floor, RamBasis::Floor)
    } else if memory.total_mib.is_none() {
        (rounded, RamBasis::Unknown)
    } else if rounded < target {
        (rounded, RamBasis::Limited)
    } else {
        (rounded, RamBasis::Target)
    };
    RamResolution {
        mib: u32::try_from(mib).unwrap_or(u32::MAX),
        basis,
    }
}

/// What an automatic allocation would be for each kind of game on this machine, for the
/// settings screens. One reading of the machine, no input from the frontend.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RamPreview {
    pub total_mib: Option<u64>,
    pub vanilla: RamResolution,
    pub lite: RamResolution,
    pub standard: RamResolution,
    pub deluxe: RamResolution,
}

pub fn preview(memory: &MemoryStatus) -> RamPreview {
    RamPreview {
        total_mib: memory.total_mib,
        vanilla: resolve(RamProfile::Vanilla, memory),
        lite: resolve(RamProfile::Lite, memory),
        standard: resolve(RamProfile::Standard, memory),
        deluxe: resolve(RamProfile::Deluxe, memory),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory(total: Option<u64>, available: Option<u64>) -> MemoryStatus {
        MemoryStatus {
            total_mib: total,
            available_mib: available,
        }
    }

    fn resolved(profile: RamProfile, total: Option<u64>) -> (u32, RamBasis) {
        let result = resolve(profile, &memory(total, None));
        (result.mib, result.basis)
    }

    #[test]
    fn a_roomy_machine_gets_the_profile_target() {
        for (profile, mib) in [
            (RamProfile::Vanilla, 4_096),
            (RamProfile::Lite, 4_096),
            (RamProfile::Standard, 6_144),
            (RamProfile::Deluxe, 8_192),
        ] {
            assert_eq!(
                resolved(profile, Some(16_384)),
                (mib, RamBasis::Target),
                "{profile:?}"
            );
        }
    }

    #[test]
    fn a_small_machine_lowers_the_heap_and_a_floor_protects_it() {
        // An 8 GiB machine keeps 2 GiB for the system, so a Deluxe pack gets 6 GiB.
        assert_eq!(
            resolved(RamProfile::Deluxe, Some(8_192)),
            (6_144, RamBasis::Limited)
        );
        assert_eq!(
            resolved(RamProfile::Standard, Some(8_192)),
            (6_144, RamBasis::Target)
        );
        // A 6 GiB machine can give a Standard pack 4 GiB.
        assert_eq!(
            resolved(RamProfile::Standard, Some(6_144)),
            (4_096, RamBasis::Limited)
        );
        // Vanilla on a 4 GiB machine keeps 2 GiB for the system.
        assert_eq!(
            resolved(RamProfile::Vanilla, Some(4_096)),
            (2_048, RamBasis::Limited)
        );
        // A pack that cannot be given its minimum is still launched at the minimum.
        assert_eq!(
            resolved(RamProfile::Standard, Some(4_096)),
            (3_072, RamBasis::Floor)
        );
        assert_eq!(
            resolved(RamProfile::Vanilla, Some(2_048)),
            (2_048, RamBasis::Floor)
        );
    }

    #[test]
    fn the_free_memory_at_this_moment_never_changes_the_heap() {
        for available in [None, Some(0), Some(300), Some(2_000), Some(1 << 22)] {
            let busy = resolve(RamProfile::Deluxe, &memory(Some(16_384), available));
            assert_eq!((busy.mib, busy.basis), (8_192, RamBasis::Target));
            let none_free = resolve(RamProfile::Standard, &memory(Some(8_192), available));
            assert_eq!((none_free.mib, none_free.basis), (6_144, RamBasis::Target));
        }
    }

    #[test]
    fn unreadable_total_memory_falls_back_to_a_safe_value() {
        for profile in [RamProfile::Vanilla, RamProfile::Lite, RamProfile::Deluxe] {
            assert_eq!(resolved(profile, None), (4_096, RamBasis::Unknown));
        }
        assert_eq!(
            resolve(RamProfile::Vanilla, &MemoryStatus::UNKNOWN).mib,
            4_096
        );
        // The free memory is not a substitute for the total.
        let only_free = resolve(RamProfile::Standard, &memory(None, Some(500)));
        assert_eq!((only_free.mib, only_free.basis), (4_096, RamBasis::Unknown));
    }

    #[test]
    fn every_result_is_a_valid_multiple_within_the_bounds() {
        for profile in [
            RamProfile::Vanilla,
            RamProfile::Lite,
            RamProfile::Standard,
            RamProfile::Deluxe,
        ] {
            assert!(profile.target_mib() <= CAP_MIB && profile.floor_mib() <= profile.target_mib());
            for total in [
                None,
                Some(1),
                Some(2_048),
                Some(4_000),
                Some(12_345),
                Some(65_536),
                Some(1 << 22),
            ] {
                let result = resolve(profile, &memory(total, None));
                let mib = u64::from(result.mib);
                assert!(
                    mib >= profile.floor_mib() && mib <= CAP_MIB,
                    "{profile:?} {total:?}"
                );
                assert_eq!(mib % STEP_MIB, 0);
                assert!((512..=262_144).contains(&result.mib));
            }
        }
    }

    #[test]
    fn profile_comes_from_the_installed_pack() {
        assert_eq!(RamProfile::of(None), RamProfile::Vanilla);
    }

    #[test]
    fn the_log_note_has_only_fixed_fields() {
        let machine = memory(Some(8_192), Some(5_000));
        let note = resolve(RamProfile::Standard, &machine).note(RamProfile::Standard, &machine);
        assert_eq!(
            note,
            "ram auto profile=Standard heap_mib=6144 basis=Target total_mib=8192"
        );
        let unknown = RamResolution {
            mib: 4_096,
            basis: RamBasis::Unknown,
        };
        assert!(
            unknown
                .note(RamProfile::Vanilla, &MemoryStatus::UNKNOWN)
                .ends_with("total_mib=unknown")
        );
    }

    #[test]
    fn the_preview_matches_the_resolver_and_serializes_for_the_frontend() {
        let machine = memory(Some(8_192), Some(5_000));
        let preview = preview(&machine);
        assert_eq!(preview.vanilla, resolve(RamProfile::Vanilla, &machine));
        assert_eq!(preview.deluxe, resolve(RamProfile::Deluxe, &machine));
        let json = serde_json::to_value(preview).unwrap_or_default();
        assert_eq!(json["totalMib"], 8_192);
        assert!(json.get("availableMib").is_none());
        assert_eq!(json["deluxe"]["mib"], 6_144);
        assert_eq!(json["deluxe"]["basis"], "limited");
        let unknown =
            serde_json::to_value(super::preview(&MemoryStatus::UNKNOWN)).unwrap_or_default();
        assert!(unknown["totalMib"].is_null() && unknown["vanilla"]["basis"] == "unknown");
    }
}
