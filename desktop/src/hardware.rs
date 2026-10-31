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
    let name = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("PRETTY_NAME="))
                .map(|value| value.trim_matches('"').to_owned())
        })
        .unwrap_or_else(|| "linux".to_owned());
    let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default();
    format!("{name} kernel={}", kernel.trim())
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
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("model name"))
                .and_then(|line| line.split_once(':'))
                .map(|(_, value)| value.trim().to_owned())
        })
        .unwrap_or_else(|| "unknown".to_owned())
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
    let text = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name))
            .and_then(|rest| rest.trim_start_matches(':').split_whitespace().next())
            .and_then(|value| value.parse::<u64>().ok())
            .map(|kib| kib * 1024)
    };
    (field("MemTotal"), field("MemAvailable"))
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
    std::fs::read_to_string("/sys/class/drm/card0/device/uevent")
        .ok()
        .map(|text| {
            let value = |key: &str| {
                text.lines()
                    .find_map(|line| line.strip_prefix(key))
                    .unwrap_or("unknown")
                    .to_owned()
            };
            format!("driver={} pci_id={}", value("DRIVER="), value("PCI_ID="))
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn gpu() -> String {
    "unknown".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

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
