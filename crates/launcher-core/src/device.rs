//! Informational device text for activation requests. It reaches Discord so the
//! member can recognize the launcher; it is never used for a decision.
use crate::panita_api::DeviceInfo;

impl DeviceInfo {
    /// The computer's name and operating system. Either may be absent.
    pub fn detect() -> Self {
        Self::new(computer_name().as_deref(), operating_system().as_deref())
    }
}

#[cfg(windows)]
fn computer_name() -> Option<String> {
    std::env::var("COMPUTERNAME").ok()
}

#[cfg(not(windows))]
fn computer_name() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
}

#[cfg(windows)]
fn operating_system() -> Option<String> {
    Some(windows_name(windows_build()).to_owned())
}

#[cfg(target_os = "linux")]
fn operating_system() -> Option<String> {
    Some(
        std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|text| pretty_name(&text))
            .unwrap_or_else(|| "Linux".to_owned()),
    )
}

#[cfg(not(any(windows, target_os = "linux")))]
fn operating_system() -> Option<String> {
    None
}

#[cfg(windows)]
fn windows_build() -> Option<u32> {
    use winsafe::{HKEY, RegistryValue, co};
    let key = HKEY::LOCAL_MACHINE
        .RegOpenKeyEx(
            Some(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion"),
            co::REG_OPTION::default(),
            co::KEY::READ,
        )
        .ok()?;
    match key.RegQueryValueEx(Some("CurrentBuildNumber")).ok()? {
        RegistryValue::Sz(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// Windows 11 reports the same major version as Windows 10; its builds start at 22000.
#[cfg(any(windows, test))]
fn windows_name(build: Option<u32>) -> &'static str {
    match build {
        Some(build) if build >= 22000 => "Windows 11",
        Some(_) => "Windows 10",
        None => "Windows",
    }
}

/// `PRETTY_NAME` of an os-release file, without its quotes.
#[cfg(any(target_os = "linux", test))]
fn pretty_name(os_release: &str) -> Option<String> {
    os_release
        .lines()
        .find_map(|line| line.trim().strip_prefix("PRETTY_NAME="))
        .map(|value| value.trim().trim_matches(['"', '\'']).trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_builds_map_to_their_marketing_name() {
        assert_eq!(windows_name(Some(26300)), "Windows 11");
        assert_eq!(windows_name(Some(22000)), "Windows 11");
        assert_eq!(windows_name(Some(21996)), "Windows 10");
        assert_eq!(windows_name(Some(19045)), "Windows 10");
        assert_eq!(windows_name(None), "Windows");
    }

    #[test]
    fn os_release_pretty_names_are_read_without_quotes() {
        let release = "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 26.04.1 LTS\"\nID=ubuntu\n";
        assert_eq!(pretty_name(release).as_deref(), Some("Ubuntu 26.04.1 LTS"));
        assert_eq!(
            pretty_name("PRETTY_NAME=CachyOS\n").as_deref(),
            Some("CachyOS")
        );
        assert_eq!(
            pretty_name("PRETTY_NAME='Arch Linux'").as_deref(),
            Some("Arch Linux")
        );
        assert_eq!(pretty_name("NAME=Ubuntu\n"), None);
        assert_eq!(pretty_name("PRETTY_NAME=\"\"\n"), None);
    }

    #[test]
    fn detected_text_is_always_safe_for_the_api() {
        let device = DeviceInfo::detect();
        for text in [device.name(), device.os()].into_iter().flatten() {
            assert!(!text.is_empty() && text.chars().count() <= 40);
            assert!(!text.chars().any(char::is_control));
            assert!(!text.contains('@') && !text.contains('<'));
        }
        #[cfg(any(windows, target_os = "linux"))]
        assert!(device.os().is_some());
    }
}
