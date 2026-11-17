//! One shared, hidden desktop entry that gives Minecraft game windows an identity.
//!
//! The game process receives `SDL_APP_ID=com.panitacraft.minecraft` (see the
//! launcher core), so the Wayland compositor groups its window under this entry
//! and shows the entry's name and icon instead of a generic gear. The entry is
//! `NoDisplay`, so it never appears in the application grid, and it is shared by
//! every instance: no file is created per instance.
use std::io::{Error, ErrorKind, Write};
use std::path::Path;

const ENTRY_ID: &str = "com.panitacraft.minecraft.desktop";
const ICON_FILE: &str = "com.panitacraft.minecraft.png";
const OWNER_LINE: &str = "X-PanitaClient-Owner=com.panitacraft.client";
const OWNED_PREFIX: &str = "[Desktop Entry]\nType=Application\nName=Minecraft\n";
/// Shown until the first game launch publishes the real Minecraft icon.
const FALLBACK_ICON: &[u8] = include_bytes!("../icons/icon.png");

fn entry(escaped_executable: &str) -> String {
    format!(
        "{OWNED_PREFIX}Exec=\"{escaped_executable}\"\nIcon=com.panitacraft.minecraft\nStartupWMClass=com.panitacraft.minecraft\nNoDisplay=true\nTerminal=false\n{OWNER_LINE}\n"
    )
}

fn launcher_owned(content: &str) -> bool {
    content.starts_with(OWNED_PREFIX) && content.lines().any(|line| line == OWNER_LINE)
}

fn write_atomically(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let directory = path
        .parent()
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "missing parent"))?;
    std::fs::create_dir_all(directory)?;
    let temporary = directory.join(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temporary)?;
        file.write_all(content)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn publish_entry(directory: &Path, expected: &str) -> std::io::Result<bool> {
    let path = directory.join(ENTRY_ID);
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err(Error::new(
                ErrorKind::AlreadyExists,
                "desktop entry is not a regular file",
            ));
        }
        Ok(_) => {
            let current = std::fs::read_to_string(&path)?;
            if current == expected {
                return Ok(false);
            }
            // An AppImage moves between versions, so a launcher-owned entry may be
            // rewritten. Anything else at this name belongs to the player.
            if !launcher_owned(&current) {
                return Err(Error::new(
                    ErrorKind::AlreadyExists,
                    "desktop entry is not launcher-owned",
                ));
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    write_atomically(&path, expected.as_bytes())?;
    Ok(true)
}

/// Publish the Panita icon until real Minecraft icons exist in the icon theme.
fn publish_fallback_icon(hicolor: &Path) -> std::io::Result<bool> {
    let minecraft = hicolor.join("256x256").join("apps").join(ICON_FILE);
    if std::fs::symlink_metadata(minecraft).is_ok() {
        return Ok(false);
    }
    let path = hicolor.join("512x512").join("apps").join(ICON_FILE);
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err(Error::new(ErrorKind::AlreadyExists, "icon is not a file"));
        }
        Ok(_) if std::fs::read(&path)? == FALLBACK_ICON => return Ok(false),
        Ok(_) | Err(_) => {}
    }
    write_atomically(&path, FALLBACK_ICON)?;
    Ok(true)
}

/// Returns whether any file changed. Failures leave the player's files untouched
/// and never block the launcher: the game then simply keeps a generic icon.
pub(super) fn publish(
    escaped_executable: &str,
    directory: &Path,
    hicolor: &Path,
) -> std::io::Result<bool> {
    let entry_changed = publish_entry(directory, &entry(escaped_executable))?;
    let icon_changed = publish_fallback_icon(hicolor)?;
    Ok(entry_changed || icon_changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn directory(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "panita-game-identity-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_else(|_| panic!("system clock"))
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap_or_else(|_| panic!("test directory"));
        path
    }

    #[test]
    fn entry_is_hidden_shared_and_matches_the_game_application_id() {
        let content = entry("/opt/Panita Client/panita-client");
        assert!(content.contains("\nNoDisplay=true\n"));
        assert!(content.contains("\nStartupWMClass=com.panitacraft.minecraft\n"));
        assert!(content.contains("\nIcon=com.panitacraft.minecraft\n"));
        assert!(content.contains("\nName=Minecraft\n"));
        assert!(content.contains("\nExec=\"/opt/Panita Client/panita-client\"\n"));
        assert!(launcher_owned(&content));
        assert_eq!(ENTRY_ID, "com.panitacraft.minecraft.desktop");
    }

    #[test]
    fn publishing_is_idempotent_replaces_only_owned_entries_and_preserves_player_files() {
        let root = directory("publish");
        let applications = root.join("applications");
        let hicolor = root.join("icons/hicolor");

        assert!(publish("/old/panita-client", &applications, &hicolor).unwrap_or(false));
        assert!(!publish("/old/panita-client", &applications, &hicolor).unwrap_or(true));
        assert_eq!(
            std::fs::read(hicolor.join("512x512/apps").join(ICON_FILE)).ok(),
            Some(FALLBACK_ICON.to_vec())
        );
        // A moved AppImage rewrites the owned entry.
        assert!(publish("/new/panita-client", &applications, &hicolor).unwrap_or(false));
        assert!(
            std::fs::read_to_string(applications.join(ENTRY_ID))
                .is_ok_and(|content| content.contains("/new/panita-client"))
        );

        // Real Minecraft icons suppress the fallback.
        let real = hicolor.join("256x256/apps");
        std::fs::create_dir_all(&real).unwrap_or_else(|_| panic!("real icons"));
        std::fs::write(real.join(ICON_FILE), b"minecraft").unwrap_or_else(|_| panic!("icon"));
        std::fs::remove_file(hicolor.join("512x512/apps").join(ICON_FILE))
            .unwrap_or_else(|_| panic!("remove fallback"));
        assert!(!publish("/new/panita-client", &applications, &hicolor).unwrap_or(true));
        assert!(!hicolor.join("512x512/apps").join(ICON_FILE).exists());

        // A different file at the entry name belongs to the player.
        std::fs::write(applications.join(ENTRY_ID), "[Desktop Entry]\nName=Mine\n")
            .unwrap_or_else(|_| panic!("custom entry"));
        assert!(publish("/new/panita-client", &applications, &hicolor).is_err());
        assert_eq!(
            std::fs::read_to_string(applications.join(ENTRY_ID))
                .ok()
                .as_deref(),
            Some("[Desktop Entry]\nName=Mine\n")
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
