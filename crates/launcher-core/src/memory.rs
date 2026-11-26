//! Physical memory of this machine. Best effort: a value that cannot be read is
//! `None`, never an error, and nothing else about the machine is read.

/// Installed and currently available physical memory, in MiB.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MemoryStatus {
    pub total_mib: Option<u64>,
    pub available_mib: Option<u64>,
}

impl MemoryStatus {
    /// Nothing could be read.
    pub const UNKNOWN: Self = Self {
        total_mib: None,
        available_mib: None,
    };

    /// Read the current values. A reading of zero is treated as unreadable.
    pub fn read() -> Self {
        let status = platform();
        Self {
            total_mib: status.total_mib.filter(|value| *value > 0),
            available_mib: status.available_mib.filter(|value| *value > 0),
        }
    }
}

#[cfg(windows)]
fn platform() -> MemoryStatus {
    const MIB: u64 = 1024 * 1024;
    match winsafe::GlobalMemoryStatusEx() {
        Ok(status) => MemoryStatus {
            total_mib: Some(status.ullTotalPhys / MIB),
            available_mib: Some(status.ullAvailPhys / MIB),
        },
        Err(_) => MemoryStatus::UNKNOWN,
    }
}

#[cfg(target_os = "linux")]
fn platform() -> MemoryStatus {
    parse_meminfo(&std::fs::read_to_string("/proc/meminfo").unwrap_or_default())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn platform() -> MemoryStatus {
    MemoryStatus::UNKNOWN
}

/// Total and available memory from the kibibyte fields of /proc/meminfo. A pure text
/// function, so every platform can test it.
#[cfg(any(target_os = "linux", test))]
fn parse_meminfo(meminfo: &str) -> MemoryStatus {
    let field = |name: &str| {
        meminfo
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|rest| rest.trim_start_matches(':').split_whitespace().next())
            .and_then(|value| value.parse::<u64>().ok())
            .map(|kib| kib / 1024)
    };
    MemoryStatus {
        total_mib: field("MemTotal"),
        available_mib: field("MemAvailable"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_total_and_available_memory_from_proc_meminfo() {
        let meminfo = "MemTotal:       16318464 kB\nMemFree:  100 kB\nMemAvailable:   9000000 kB\n";
        assert_eq!(
            parse_meminfo(meminfo),
            MemoryStatus {
                total_mib: Some(15936),
                available_mib: Some(8789),
            }
        );
        assert_eq!(parse_meminfo("garbage"), MemoryStatus::UNKNOWN);
        assert_eq!(
            parse_meminfo("MemTotal: 2048 kB\n").available_mib,
            None,
            "a missing field stays unknown"
        );
    }

    #[test]
    fn reads_the_machine_without_failing() {
        let status = MemoryStatus::read();
        assert!(
            status.total_mib.is_some_and(|value| value > 0),
            "{status:?}"
        );
    }
}
