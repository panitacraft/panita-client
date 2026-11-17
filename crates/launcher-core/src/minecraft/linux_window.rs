//! Linux game window integration: a stable application identity for the desktop
//! shell and window decorations on native Wayland.
//!
//! Minecraft 26.x renders through SDL3 and uses native Wayland. Mutter (GNOME)
//! offers no server-side decorations, so SDL3 loads `libdecor` to draw the title
//! bar. Debian and Arch packages depend on it. An AppImage cannot declare
//! dependencies, so it carries a copy that is published to the launcher's data
//! directory and exposed to the game only when the host lacks libdecor.
// Only Linux launches the game through this module; other platforms compile the
// platform-neutral planning code solely to run its unit tests.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Wayland `app_id` / X11 class that matches the launcher's shared Minecraft
/// desktop entry. SDL3 lets `SDL_APP_ID` override the identifier set by the game.
pub(super) const APP_ID: &str = "com.panitacraft.minecraft";

const LIBRARY_NAME: &str = "libdecor-0.so.0";
const PLUGIN_DIRECTORY: &str = "plugins-1";
const PLUGIN_NAME: &str = "libdecor-cairo.so";
const HOST_LIBRARY_DIRECTORIES: [&str; 6] = [
    "/usr/lib/x86_64-linux-gnu",
    "/lib/x86_64-linux-gnu",
    "/usr/lib64",
    "/usr/lib",
    "/lib64",
    "/lib",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct HostLibdecor {
    pub(super) library: bool,
    pub(super) plugin: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Decorations {
    /// The host provides libdecor and a plugin; leave the game environment alone.
    Host,
    /// The host has libdecor but no plugin; add only the bundled plugin directory.
    BundledPlugin,
    /// The host has no libdecor; expose the bundled library and plugin.
    BundledLibrary,
}

pub(super) fn decorations(host: HostLibdecor) -> Decorations {
    match (host.library, host.plugin) {
        (true, true) => Decorations::Host,
        (true, false) => Decorations::BundledPlugin,
        (false, _) => Decorations::BundledLibrary,
    }
}

fn plugin_directory_has_plugin(directory: &Path) -> bool {
    std::fs::read_dir(directory).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().ends_with(".so"))
    })
}

pub(super) fn host_libdecor_in(directories: &[&Path]) -> HostLibdecor {
    HostLibdecor {
        library: directories
            .iter()
            .any(|directory| directory.join(LIBRARY_NAME).exists()),
        plugin: directories.iter().any(|directory| {
            plugin_directory_has_plugin(&directory.join("libdecor").join(PLUGIN_DIRECTORY))
        }),
    }
}

pub(super) fn environment_for(support: Decorations, directory: &Path) -> Vec<(OsString, OsString)> {
    let plugins = directory.join(PLUGIN_DIRECTORY);
    match support {
        Decorations::Host => Vec::new(),
        Decorations::BundledPlugin => vec![("LIBDECOR_PLUGIN_DIR".into(), plugins.into())],
        Decorations::BundledLibrary => vec![
            // A single narrow directory that holds only libdecor, so no host
            // library can be shadowed. The launcher's AppImage paths stay removed.
            ("LD_LIBRARY_PATH".into(), directory.as_os_str().to_owned()),
            ("LIBDECOR_PLUGIN_DIR".into(), plugins.into()),
        ],
    }
}

#[cfg(target_os = "linux")]
mod runtime_copy {
    use std::fs;
    use std::io::{Error, ErrorKind, Write};
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    use std::path::{Path, PathBuf};

    use sha2::{Digest, Sha256};
    use uuid::Uuid;

    use super::{LIBRARY_NAME, PLUGIN_DIRECTORY, PLUGIN_NAME};

    const MAX_FILE: u64 = 8 * 1024 * 1024;

    fn private_directory(path: &Path) -> std::io::Result<()> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_dir() => Ok(()),
            Ok(_) => Err(Error::new(
                ErrorKind::AlreadyExists,
                "library directory is not a regular directory",
            )),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                fs::DirBuilder::new().mode(0o700).create(path)
            }
            Err(error) => Err(error),
        }
    }

    fn digest(bytes: &[u8]) -> [u8; 32] {
        Sha256::digest(bytes).into()
    }

    fn matches(path: &Path, expected: &[u8; 32]) -> bool {
        let Ok(metadata) = fs::symlink_metadata(path) else {
            return false;
        };
        metadata.file_type().is_file()
            && metadata.len() <= MAX_FILE
            && fs::read(path).is_ok_and(|bytes| &digest(&bytes) == expected)
    }

    fn publish_file(source: &Path, destination: &Path, mode: u32) -> std::io::Result<()> {
        let metadata = fs::symlink_metadata(source)?;
        if !metadata.file_type().is_file() || metadata.len() > MAX_FILE {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "bundled file is unusable",
            ));
        }
        let bytes = fs::read(source)?;
        if matches(destination, &digest(&bytes)) {
            return Ok(());
        }
        let name = destination
            .file_name()
            .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "missing file name"))?;
        let temporary = destination.with_file_name(format!(
            ".{}.{}.tmp",
            name.to_string_lossy(),
            Uuid::new_v4().simple()
        ));
        let result = (|| {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(mode)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            // Replacing atomically keeps a running game's mapped library intact.
            fs::rename(&temporary, destination)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    /// Copy the AppImage's libdecor files below `destination` and return it.
    /// Files are verified by content, replaced atomically and owner-only.
    pub(super) fn publish(source: &Path, destination: &Path) -> std::io::Result<PathBuf> {
        let parent = destination
            .parent()
            .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "missing parent"))?;
        private_directory(parent)?;
        private_directory(destination)?;
        private_directory(&destination.join(PLUGIN_DIRECTORY))?;
        publish_file(
            &source.join(LIBRARY_NAME),
            &destination.join(LIBRARY_NAME),
            0o700,
        )?;
        publish_file(
            &source.join(PLUGIN_DIRECTORY).join(PLUGIN_NAME),
            &destination.join(PLUGIN_DIRECTORY).join(PLUGIN_NAME),
            0o700,
        )?;
        Ok(destination.to_owned())
    }
}

/// The Minecraft icon, copied from the game's own verified assets, so the shell
/// can show it for the helper desktop entry. Nothing is bundled with the launcher.
#[cfg(target_os = "linux")]
mod game_icons {
    use std::collections::HashMap;
    use std::fs;
    use std::io::{Error, ErrorKind, Write};
    use std::path::{Path, PathBuf};

    use serde::Deserialize;
    use sha1::{Digest, Sha1};
    use uuid::Uuid;

    const SIZES: [u32; 5] = [16, 32, 48, 128, 256];
    const ICON_FILE: &str = "com.panitacraft.minecraft.png";
    const MAX_INDEX: u64 = 32 * 1024 * 1024;
    const MAX_ICON: u64 = 1024 * 1024;
    const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

    #[derive(Deserialize)]
    struct Index {
        objects: HashMap<String, Object>,
    }

    #[derive(Deserialize)]
    struct Object {
        hash: String,
    }

    fn read_limited(path: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_file() || metadata.len() > limit {
            return Err(Error::new(ErrorKind::InvalidData, "unusable file"));
        }
        fs::read(path)
    }

    fn verified_icon(assets: &Path, index: &Index, size: u32) -> Option<Vec<u8>> {
        let hash = &index
            .objects
            .get(&format!("icons/icon_{size}x{size}.png"))?
            .hash;
        if hash.len() != 40 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let bytes = read_limited(
            &assets.join("objects").join(&hash[..2]).join(hash),
            MAX_ICON,
        )
        .ok()?;
        let actual = Sha1::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        (actual.eq_ignore_ascii_case(hash) && bytes.starts_with(&PNG_SIGNATURE)).then_some(bytes)
    }

    fn publish_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if !metadata.file_type().is_file() => {
                return Err(Error::new(ErrorKind::AlreadyExists, "icon is not a file"));
            }
            Ok(_) if fs::read(path).is_ok_and(|current| current == bytes) => return Ok(()),
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let directory = path
            .parent()
            .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "missing parent"))?;
        fs::create_dir_all(directory)?;
        let temporary = directory.join(format!(".{ICON_FILE}.{}.tmp", Uuid::new_v4().simple()));
        let result = (|| {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    /// Publish every available size below `data_home/icons/hicolor`. Returns the
    /// number of sizes now present. A missing or unverifiable object is skipped.
    pub(super) fn publish(assets: &Path, index: &Path, data_home: &Path) -> std::io::Result<usize> {
        let index: Index = serde_json::from_slice(&read_limited(index, MAX_INDEX)?)
            .map_err(|_| Error::new(ErrorKind::InvalidData, "asset index is malformed"))?;
        let hicolor = data_home.join("icons").join("hicolor");
        let mut published = 0;
        for size in SIZES {
            let Some(bytes) = verified_icon(assets, &index, size) else {
                continue;
            };
            let target = hicolor
                .join(format!("{size}x{size}"))
                .join("apps")
                .join(ICON_FILE);
            if publish_file(&target, &bytes).is_ok() {
                published += 1;
            }
        }
        if published > 0 {
            // The Panita fallback written by the desktop integration is replaced by
            // the real icon. Only this launcher-named file is ever removed.
            let fallback = hicolor.join("512x512").join("apps").join(ICON_FILE);
            if fs::symlink_metadata(&fallback).is_ok_and(|metadata| metadata.file_type().is_file())
            {
                let _ = fs::remove_file(fallback);
            }
        }
        Ok(published)
    }

    pub(super) fn data_home() -> Option<PathBuf> {
        let absolute = |value: std::ffi::OsString| {
            let path = PathBuf::from(value);
            path.is_absolute().then_some(path)
        };
        std::env::var_os("XDG_DATA_HOME")
            .and_then(absolute)
            .or_else(|| {
                std::env::var_os("HOME")
                    .and_then(absolute)
                    .map(|home| home.join(".local").join("share"))
            })
    }
}

/// Best effort: a failure here never affects the game launch.
#[cfg(target_os = "linux")]
pub(super) fn publish_minecraft_icons(paths: &crate::paths::AppPaths, asset_index: &str) {
    let Some(data_home) = game_icons::data_home() else {
        return;
    };
    let _ = game_icons::publish(
        &paths.root().join("assets"),
        &paths.root().join(asset_index),
        &data_home,
    );
}

/// Directory inside the running AppImage that holds the bundled libdecor.
#[cfg(target_os = "linux")]
fn bundled_directory() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let directory = executable
        .parent()?
        .parent()?
        .join("lib")
        .join("panita")
        .join("libdecor");
    directory.join(LIBRARY_NAME).is_file().then_some(directory)
}

#[cfg(target_os = "linux")]
fn appimage_decorations(paths: &crate::paths::AppPaths) -> Vec<(OsString, OsString)> {
    let directories: Vec<&Path> = HOST_LIBRARY_DIRECTORIES.iter().map(Path::new).collect();
    let support = decorations(host_libdecor_in(&directories));
    if support == Decorations::Host {
        return Vec::new();
    }
    let Some(source) = bundled_directory() else {
        return Vec::new();
    };
    let destination = paths.root().join("runtime-libs").join("libdecor");
    match runtime_copy::publish(&source, &destination) {
        Ok(directory) => environment_for(support, &directory),
        // Without a verified private copy the game still starts, only without a title bar.
        Err(_) => Vec::new(),
    }
}

/// Environment that gives the game window a stable identity and, inside an
/// AppImage, working decorations. Deb and Arch rely on package dependencies.
#[cfg(target_os = "linux")]
pub(super) fn game_window_environment(paths: &crate::paths::AppPaths) -> Vec<(OsString, OsString)> {
    let mut environment = vec![("SDL_APP_ID".into(), APP_ID.into())];
    if std::env::var_os("APPIMAGE").is_some() {
        environment.extend(appimage_decorations(paths));
    }
    environment
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "panita-linux-window-{label}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&path).unwrap_or_else(|_| panic!("create {label}"));
        path
    }

    #[test]
    fn host_support_decides_what_the_game_receives() {
        let both = HostLibdecor {
            library: true,
            plugin: true,
        };
        let no_plugin = HostLibdecor {
            library: true,
            plugin: false,
        };
        let none = HostLibdecor {
            library: false,
            plugin: false,
        };
        assert_eq!(decorations(both), Decorations::Host);
        assert_eq!(decorations(no_plugin), Decorations::BundledPlugin);
        assert_eq!(decorations(none), Decorations::BundledLibrary);
        let directory = Path::new("/data/runtime-libs/libdecor");
        assert!(environment_for(Decorations::Host, directory).is_empty());
        assert_eq!(
            environment_for(Decorations::BundledPlugin, directory),
            [(
                OsString::from("LIBDECOR_PLUGIN_DIR"),
                directory.join("plugins-1").into_os_string()
            )]
        );
        let library = environment_for(Decorations::BundledLibrary, directory);
        assert_eq!(
            library[0],
            (
                OsString::from("LD_LIBRARY_PATH"),
                directory.as_os_str().to_owned()
            )
        );
        assert_eq!(library.len(), 2);
    }

    #[test]
    fn host_detection_needs_both_the_library_and_a_plugin() {
        let host = temporary("host");
        assert_eq!(
            host_libdecor_in(&[host.as_path()]),
            HostLibdecor {
                library: false,
                plugin: false
            }
        );
        std::fs::write(host.join(LIBRARY_NAME), b"library").unwrap_or_else(|_| panic!("library"));
        assert_eq!(
            host_libdecor_in(&[host.as_path()]),
            HostLibdecor {
                library: true,
                plugin: false
            }
        );
        let plugins = host.join("libdecor").join(PLUGIN_DIRECTORY);
        std::fs::create_dir_all(&plugins).unwrap_or_else(|_| panic!("plugins"));
        std::fs::write(plugins.join("README"), b"not a plugin").unwrap_or_else(|_| panic!("note"));
        assert!(!host_libdecor_in(&[host.as_path()]).plugin);
        std::fs::write(plugins.join(PLUGIN_NAME), b"plugin").unwrap_or_else(|_| panic!("plugin"));
        assert_eq!(
            host_libdecor_in(&[host.as_path()]),
            HostLibdecor {
                library: true,
                plugin: true
            }
        );
        let _ = std::fs::remove_dir_all(host);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn runtime_copy_is_private_verified_and_replaced_atomically() {
        use std::os::unix::fs::PermissionsExt;

        let root = temporary("copy");
        let source = root.join("bundle");
        std::fs::create_dir_all(source.join(PLUGIN_DIRECTORY)).unwrap_or_else(|_| panic!("source"));
        std::fs::write(source.join(LIBRARY_NAME), b"library one")
            .unwrap_or_else(|_| panic!("library"));
        std::fs::write(
            source.join(PLUGIN_DIRECTORY).join(PLUGIN_NAME),
            b"plugin one",
        )
        .unwrap_or_else(|_| panic!("plugin"));
        let destination = root.join("runtime-libs").join("libdecor");

        let published =
            runtime_copy::publish(&source, &destination).unwrap_or_else(|_| panic!("publish"));
        assert_eq!(published, destination);
        let library = destination.join(LIBRARY_NAME);
        let mode = |path: &Path| std::fs::metadata(path).map(|m| m.permissions().mode() & 0o777);
        assert_eq!(
            std::fs::read(&library).ok().as_deref(),
            Some(&b"library one"[..])
        );
        assert_eq!(mode(&library).ok(), Some(0o700));
        assert_eq!(mode(&destination).ok(), Some(0o700));

        // Unchanged content is left alone; changed content is replaced.
        let inode = std::os::unix::fs::MetadataExt::ino(
            &std::fs::metadata(&library).unwrap_or_else(|_| panic!("meta")),
        );
        runtime_copy::publish(&source, &destination).unwrap_or_else(|_| panic!("republish"));
        assert_eq!(
            std::os::unix::fs::MetadataExt::ino(
                &std::fs::metadata(&library).unwrap_or_else(|_| panic!("meta"))
            ),
            inode
        );
        std::fs::write(source.join(LIBRARY_NAME), b"library two")
            .unwrap_or_else(|_| panic!("update"));
        runtime_copy::publish(&source, &destination).unwrap_or_else(|_| panic!("update publish"));
        assert_eq!(
            std::fs::read(&library).ok().as_deref(),
            Some(&b"library two"[..])
        );
        let leftovers = std::fs::read_dir(&destination).map(|entries| {
            entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
                .count()
        });
        assert_eq!(leftovers.ok(), Some(0));

        // A redirected directory is never followed.
        let redirected = root.join("redirected");
        std::fs::create_dir_all(&redirected).unwrap_or_else(|_| panic!("redirected"));
        let linked = root.join("linked").join("libdecor");
        std::fs::create_dir_all(linked.parent().unwrap_or(&root))
            .unwrap_or_else(|_| panic!("linked parent"));
        std::os::unix::fs::symlink(&redirected, &linked).unwrap_or_else(|_| panic!("symlink"));
        assert!(runtime_copy::publish(&source, &linked).is_err());
        assert!(!redirected.join(LIBRARY_NAME).exists());
        let _ = std::fs::remove_dir_all(root);
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn minecraft_icons_come_from_verified_asset_objects_and_replace_the_fallback() {
        use sha1::{Digest, Sha1};

        let root = temporary("icons");
        let assets = root.join("assets");
        let png = |size: u8| {
            let mut bytes = vec![137, 80, 78, 71, 13, 10, 26, 10];
            bytes.extend([size; 8]);
            bytes
        };
        let hex = |bytes: &[u8]| {
            Sha1::digest(bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        };
        let mut objects = Vec::new();
        for size in [16_u8, 32] {
            let bytes = png(size);
            let hash = hex(&bytes);
            let directory = assets.join("objects").join(&hash[..2]);
            std::fs::create_dir_all(&directory).unwrap_or_else(|_| panic!("objects"));
            std::fs::write(directory.join(&hash), &bytes).unwrap_or_else(|_| panic!("object"));
            objects.push(format!(
                "\"icons/icon_{size}x{size}.png\":{{\"hash\":\"{hash}\",\"size\":{}}}",
                bytes.len()
            ));
        }
        // Tampered object: the content no longer matches its name.
        let tampered = "a".repeat(40);
        let directory = assets.join("objects").join("aa");
        std::fs::create_dir_all(&directory).unwrap_or_else(|_| panic!("tampered"));
        std::fs::write(directory.join(&tampered), png(9)).unwrap_or_else(|_| panic!("object"));
        objects.push(format!(
            "\"icons/icon_48x48.png\":{{\"hash\":\"{tampered}\",\"size\":16}}"
        ));
        std::fs::create_dir_all(assets.join("indexes")).unwrap_or_else(|_| panic!("indexes"));
        let index = assets.join("indexes").join("test.json");
        std::fs::write(&index, format!("{{\"objects\":{{{}}}}}", objects.join(",")))
            .unwrap_or_else(|_| panic!("index"));

        let data = root.join("data");
        let fallback = data
            .join("icons/hicolor/512x512/apps")
            .join("com.panitacraft.minecraft.png");
        std::fs::create_dir_all(fallback.parent().unwrap_or(&data))
            .unwrap_or_else(|_| panic!("fallback directory"));
        std::fs::write(&fallback, b"panita fallback").unwrap_or_else(|_| panic!("fallback"));

        let published =
            game_icons::publish(&assets, &index, &data).unwrap_or_else(|_| panic!("publish"));
        assert_eq!(published, 2);
        for (size, byte) in [(16, 16_u8), (32, 32)] {
            let icon = data
                .join(format!("icons/hicolor/{size}x{size}/apps"))
                .join("com.panitacraft.minecraft.png");
            assert_eq!(std::fs::read(icon).ok(), Some(png(byte)));
        }
        assert!(!data.join("icons/hicolor/48x48").exists());
        assert!(!fallback.exists());
        // A second run is idempotent.
        assert_eq!(game_icons::publish(&assets, &index, &data).ok(), Some(2));
        let _ = std::fs::remove_dir_all(root);
    }
}
