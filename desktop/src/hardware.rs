//! Hardware and system facts for the head of each session log. Best effort: a
//! missing value is reported as `unknown`, never as an error. No serial numbers,
//! user names, network identifiers or locations are read.
use std::path::Path;

const MIB: u64 = 1024 * 1024;

pub(super) fn lines(data_root: &Path) -> Vec<String> {
    let (total, available) = memory();
    let disk = (
        fs2::total_space(data_root).ok(),
        fs2::available_space(data_root).ok(),
    );
    vec![
        format!("system os={}", os()),
        format!(
            "cpu name={} logical_cores={}",
            cpu_name(),
            std::thread::available_parallelism().map_or(0, usize::from)
        ),
        format!(
            "memory total_mib={} available_mib={}",
            mib(total),
            mib(available)
        ),
        format!("gpu name={}", gpu()),
        format!("disk total_mib={} free_mib={}", mib(disk.0), mib(disk.1)),
    ]
}

fn mib(bytes: Option<u64>) -> String {
    bytes.map_or_else(|| "unknown".to_owned(), |value| (value / MIB).to_string())
}

#[cfg(windows)]
fn os() -> String {
    let version = windows_version::OsVersion::current();
    format!(
        "windows {}.{}.{}",
        version.major, version.minor, version.build
    )
}

#[cfg(target_os = "linux")]
fn os() -> String {
    let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    linux_text::os(&release, &kernel)
}

#[cfg(not(any(windows, target_os = "linux")))]
fn os() -> String {
    std::env::consts::OS.to_owned()
}

#[cfg(windows)]
fn cpu_name() -> String {
    windows_registry::LOCAL_MACHINE
        .open("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0")
        .and_then(|key| key.get_string("ProcessorNameString"))
        .map_or_else(|_| "unknown".to_owned(), |name| name.trim().to_owned())
}

#[cfg(target_os = "linux")]
fn cpu_name() -> String {
    linux_text::cpu_name(&std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn cpu_name() -> String {
    "unknown".to_owned()
}

#[cfg(windows)]
fn memory() -> (Option<u64>, Option<u64>) {
    match winsafe::GlobalMemoryStatusEx() {
        Ok(status) => (Some(status.ullTotalPhys), Some(status.ullAvailPhys)),
        Err(_) => (None, None),
    }
}

#[cfg(target_os = "linux")]
fn memory() -> (Option<u64>, Option<u64>) {
    linux_text::memory(&std::fs::read_to_string("/proc/meminfo").unwrap_or_default())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn memory() -> (Option<u64>, Option<u64>) {
    (None, None)
}

#[cfg(windows)]
fn gpu() -> String {
    // The first display adapter; enough to spot integrated-only or very old hardware.
    windows_registry::LOCAL_MACHINE
        .open("SYSTEM\\CurrentControlSet\\Control\\Class\\{4d36e968-e325-11ce-bfc1-08002be10318}\\0000")
        .and_then(|key| key.get_string("DriverDesc"))
        .map_or_else(|_| "unknown".to_owned(), |name| name.trim().to_owned())
}

#[cfg(target_os = "linux")]
fn gpu() -> String {
    // Hybrid laptops expose several cards; list each one that has a kernel driver.
    let cards: Vec<String> = (0..4)
        .filter_map(|index| {
            std::fs::read_to_string(format!("/sys/class/drm/card{index}/device/uevent")).ok()
        })
        .filter_map(|text| linux_text::gpu(&text))
        .collect();
    if cards.is_empty() {
        "unknown".to_owned()
    } else {
        cards.join(", ")
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn gpu() -> String {
    "unknown".to_owned()
}

/// Parsers for Linux system files. Pure text functions so every platform can test them.
#[cfg(any(target_os = "linux", test))]
mod linux_text {
    pub(super) fn os(release: &str, kernel: &str) -> String {
        let name = release
            .lines()
            .find_map(|line| line.strip_prefix("PRETTY_NAME="))
            .map_or("linux", |value| value.trim().trim_matches('"'));
        format!("{name} kernel={}", kernel.trim())
    }

    pub(super) fn cpu_name(cpuinfo: &str) -> String {
        cpuinfo
            .lines()
            .find(|line| line.starts_with("model name"))
            .and_then(|line| line.split_once(':'))
            .map_or_else(
                || "unknown".to_owned(),
                |(_, value)| value.trim().to_owned(),
            )
    }

    /// Total and available bytes from the kibibyte fields of /proc/meminfo.
    pub(super) fn memory(meminfo: &str) -> (Option<u64>, Option<u64>) {
        let field = |name: &str| {
            meminfo
                .lines()
                .find_map(|line| line.strip_prefix(name))
                .and_then(|rest| rest.trim_start_matches(':').split_whitespace().next())
                .and_then(|value| value.parse::<u64>().ok())
                .map(|kib| kib * 1024)
        };
        (field("MemTotal"), field("MemAvailable"))
    }

    pub(super) fn gpu(uevent: &str) -> Option<String> {
        let value = |key: &str| uevent.lines().find_map(|line| line.strip_prefix(key));
        let driver = value("DRIVER=")?;
        Some(format!(
            "{driver} ({})",
            value("PCI_ID=").unwrap_or("unknown")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ubuntu_style_system_files() {
        let release = "NAME=\"Ubuntu\"
PRETTY_NAME=\"Ubuntu 26.04 LTS\"
ID=ubuntu
";
        assert_eq!(
            linux_text::os(
                release,
                "6.14.0-generic
"
            ),
            "Ubuntu 26.04 LTS kernel=6.14.0-generic"
        );
        assert_eq!(linux_text::os("", ""), "linux kernel=");
        let cpuinfo = "processor	: 0
model name	: AMD Ryzen 7 5800X 8-Core Processor
";
        assert_eq!(
            linux_text::cpu_name(cpuinfo),
            "AMD Ryzen 7 5800X 8-Core Processor"
        );
        assert_eq!(linux_text::cpu_name("Hardware : x"), "unknown");
        let meminfo = "MemTotal:       16318464 kB
MemFree:  100 kB
MemAvailable:   9000000 kB
";
        assert_eq!(
            linux_text::memory(meminfo),
            (Some(16318464 * 1024), Some(9000000 * 1024))
        );
        assert_eq!(linux_text::memory("garbage"), (None, None));
        assert_eq!(
            linux_text::gpu(
                "DRIVER=nvidia
PCI_ID=10DE:2684
"
            )
            .as_deref(),
            Some("nvidia (10DE:2684)")
        );
        assert_eq!(
            linux_text::gpu(
                "PCI_SLOT_NAME=0000:00:02.0
"
            ),
            None
        );
    }

    #[test]
    fn describes_the_machine_without_failing() {
        let text = lines(&std::env::temp_dir()).join("\n");
        for key in [
            "system os=",
            "cpu name=",
            "memory total_mib=",
            "gpu name=",
            "disk total_mib=",
        ] {
            assert!(text.contains(key), "{key}");
        }
        assert!(!text.contains("total_mib=unknown"), "{text}");
    }
}
