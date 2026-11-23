//! Read-only access to one instance's `minecraft/screenshots` folder for the gallery.
//!
//! Callers name an instance and a bare file name; they never supply a path. Every
//! component is resolved through no-follow directory handles from the instance root,
//! so a redirected `minecraft` or `screenshots` folder, or a linked screenshot, is
//! rejected instead of read. Only regular `.png` files, Minecraft's own format, qualify.
use std::{io::Read, path::PathBuf, time::UNIX_EPOCH};

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use serde::Serialize;
use uuid::Uuid;

use crate::{
    error::{CoreError, ErrorCode},
    paths::AppPaths,
    storage::map_private_error,
};

/// Newest screenshots returned by one listing. Older ones stay on disk.
pub const MAX_LISTED: usize = 500;
/// Directory entries examined per listing, so a huge folder cannot stall the launcher.
const MAX_SCANNED: usize = 20_000;
/// Largest image read into memory, for the gallery, the clipboard or both.
pub const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
/// Largest image decoded for the clipboard (an 8K by 8K capture).
const MAX_PIXELS: u64 = 8192 * 8192;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Screenshot {
    pub name: String,
    pub size: u64,
    /// Milliseconds since the Unix epoch; zero when the file system reports none.
    pub modified_ms: u64,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotList {
    /// Newest first.
    pub screenshots: Vec<Screenshot>,
    /// More screenshots exist than were listed.
    pub truncated: bool,
}

/// An image decoded to 8-bit RGBA, ready for the clipboard.
#[derive(Debug)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

fn invalid() -> CoreError {
    CoreError::new(ErrorCode::InvalidData)
}

fn io(error: std::io::Error) -> CoreError {
    crate::storage::map_io_error(error)
}

/// A single, plain `.png` file name that is safe on Windows and Unix alike.
fn screenshot_name(name: &str) -> Result<&str, CoreError> {
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return Err(invalid());
    };
    let reserved = {
        let first = stem
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        matches!(first.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ["COM", "LPT"].iter().any(|prefix| {
                first.strip_prefix(prefix).is_some_and(|digit| {
                    matches!(digit, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
                })
            })
    };
    if !extension.eq_ignore_ascii_case("png")
        || stem.is_empty()
        || name.len() > 255
        || reserved
        || name.ends_with([' ', '.'])
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
    {
        return Err(invalid());
    }
    Ok(name)
}

/// Open a direct child directory without following links. Absent is `None`.
fn child_directory(parent: &Dir, name: &str) -> Result<Option<Dir>, CoreError> {
    match parent.symlink_metadata(name) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(invalid()),
        Ok(_) => parent.open_dir_nofollow(name).map(Some).map_err(io),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io(error)),
    }
}

/// The screenshots folder and its path, or `None` when the instance has none yet.
fn screenshots(paths: &AppPaths, instance: Uuid) -> Result<Option<(PathBuf, Dir)>, CoreError> {
    let root = match paths.instance(instance) {
        Ok(root) => root,
        // Never installed: the gallery is simply empty.
        Err(error) if error.code() == ErrorCode::InstanceMissing => return Ok(None),
        Err(error) => return Err(error),
    };
    let private = private_fs::PrivateDir::open(&root).map_err(map_private_error)?;
    let instance_directory = super::world_backup::capability(&private)?;
    let Some(game) = child_directory(&instance_directory, "minecraft")? else {
        return Ok(None);
    };
    let Some(directory) = child_directory(&game, "screenshots")? else {
        return Ok(None);
    };
    Ok(Some((
        root.join("minecraft").join("screenshots"),
        directory,
    )))
}

/// Open one screenshot, which must be a regular file and not a link.
fn open_screenshot(directory: &Dir, name: &str) -> Result<cap_std::fs::File, CoreError> {
    let metadata = directory.symlink_metadata(name).map_err(io)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(invalid());
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = directory.open_with(name, &options).map_err(io)?;
    let opened = file.metadata().map_err(io)?;
    if !opened.is_file() || opened.len() > MAX_IMAGE_BYTES {
        return Err(invalid());
    }
    Ok(file)
}

/// The instance's screenshots, newest first. A missing folder is an empty gallery.
pub fn list(paths: &AppPaths, instance: Uuid) -> Result<ScreenshotList, CoreError> {
    let Some((_, directory)) = screenshots(paths, instance)? else {
        return Ok(ScreenshotList::default());
    };
    let mut found = Vec::new();
    let mut scanned = 0_usize;
    for entry in directory.entries().map_err(io)? {
        scanned += 1;
        if scanned > MAX_SCANNED {
            break;
        }
        let Ok(name) = entry.map_err(io)?.file_name().into_string() else {
            continue;
        };
        if screenshot_name(&name).is_err() {
            continue;
        }
        // Linked, redirected or non-file entries are left out of the gallery.
        let Ok(metadata) = directory.symlink_metadata(&name) else {
            continue;
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            continue;
        }
        let modified_ms = metadata
            .modified()
            .ok()
            .and_then(|time| time.into_std().duration_since(UNIX_EPOCH).ok())
            .map_or(0, |duration| {
                u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
            });
        found.push(Screenshot {
            name,
            size: metadata.len(),
            modified_ms,
        });
    }
    found.sort_by(|a, b| {
        b.modified_ms
            .cmp(&a.modified_ms)
            .then_with(|| b.name.cmp(&a.name))
    });
    let truncated = found.len() > MAX_LISTED || scanned > MAX_SCANNED;
    found.truncate(MAX_LISTED);
    Ok(ScreenshotList {
        screenshots: found,
        truncated,
    })
}

/// The validated file contents, for showing the image in the gallery.
pub fn read(paths: &AppPaths, instance: Uuid, name: &str) -> Result<Vec<u8>, CoreError> {
    let name = screenshot_name(name)?;
    let (_, directory) = screenshots(paths, instance)?.ok_or_else(invalid)?;
    let file = open_screenshot(&directory, name)?;
    let mut bytes = Vec::new();
    file.take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(invalid());
    }
    Ok(bytes)
}

/// The path of a validated screenshot, for handing to the system viewer or file manager.
/// The check happens just before use; the player's own folder is not a hostile boundary.
pub fn locate(paths: &AppPaths, instance: Uuid, name: &str) -> Result<PathBuf, CoreError> {
    let name = screenshot_name(name)?;
    let (path, directory) = screenshots(paths, instance)?.ok_or_else(invalid)?;
    open_screenshot(&directory, name)?;
    Ok(path.join(name))
}

/// The screenshot decoded to RGBA. Oversized or malformed images are rejected.
pub fn read_rgba(paths: &AppPaths, instance: Uuid, name: &str) -> Result<Rgba, CoreError> {
    decode_png(&read(paths, instance, name)?)
}

fn decode_png(bytes: &[u8]) -> Result<Rgba, CoreError> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|_| invalid())?;
    let (width, height) = reader.info().size();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(invalid());
    }
    let mut buffer = vec![0; reader.output_buffer_size().ok_or_else(invalid)?];
    let frame = reader.next_frame(&mut buffer).map_err(|_| invalid())?;
    let data = buffer.get(..frame.buffer_size()).ok_or_else(invalid)?;
    let pixel_count = width as usize * height as usize;
    let mut pixels = Vec::with_capacity(pixel_count * 4);
    match frame.color_type {
        png::ColorType::Rgba => pixels.extend_from_slice(data),
        png::ColorType::Rgb => data.chunks_exact(3).for_each(|p| {
            pixels.extend_from_slice(&[p[0], p[1], p[2], 255]);
        }),
        png::ColorType::GrayscaleAlpha => data.chunks_exact(2).for_each(|p| {
            pixels.extend_from_slice(&[p[0], p[0], p[0], p[1]]);
        }),
        png::ColorType::Grayscale => data.iter().for_each(|&g| {
            pixels.extend_from_slice(&[g, g, g, 255]);
        }),
        png::ColorType::Indexed => return Err(invalid()),
    }
    if pixels.len() != pixel_count * 4 {
        return Err(invalid());
    }
    Ok(Rgba {
        width,
        height,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (AppPaths, Uuid, PathBuf) {
        let (paths, id) = super::super::play::tests::registered_instance();
        let game = paths.game_directory(id).unwrap_or_else(|_| panic!("game"));
        (paths, id, game)
    }

    fn encode(
        width: u32,
        height: u32,
        color: png::ColorType,
        depth: png::BitDepth,
        data: &[u8],
    ) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(color);
        encoder.set_depth(depth);
        let mut writer = encoder.write_header().unwrap_or_else(|_| panic!("header"));
        writer
            .write_image_data(data)
            .unwrap_or_else(|_| panic!("data"));
        writer.finish().unwrap_or_else(|_| panic!("finish"));
        out
    }

    fn cleanup(paths: &AppPaths) {
        std::fs::remove_dir_all(paths.root()).unwrap_or_else(|_| panic!("cleanup"));
    }

    #[test]
    fn a_missing_folder_or_instance_is_an_empty_gallery() {
        let (paths, id, game) = fixture();
        assert_eq!(list(&paths, id).ok(), Some(ScreenshotList::default()));
        std::fs::create_dir_all(game.join("screenshots")).unwrap_or_else(|_| panic!("folder"));
        assert_eq!(list(&paths, id).ok(), Some(ScreenshotList::default()));
        assert_eq!(
            list(&paths, Uuid::new_v4()).ok(),
            Some(ScreenshotList::default())
        );
        cleanup(&paths);
    }

    #[test]
    fn the_listing_holds_only_regular_png_files_newest_first() {
        let (paths, id, game) = fixture();
        let folder = game.join("screenshots");
        std::fs::create_dir_all(folder.join("nested.png")).unwrap_or_else(|_| panic!("dir"));
        for (name, seconds) in [("old.png", 10_u64), ("new.PNG", 30), ("mid.png", 20)] {
            let file = std::fs::File::create(folder.join(name)).unwrap_or_else(|_| panic!("file"));
            file.set_modified(UNIX_EPOCH + std::time::Duration::from_secs(seconds))
                .unwrap_or_else(|_| panic!("time"));
        }
        for ignored in ["notes.txt", "photo.jpg", "CON.png", "readme"] {
            std::fs::write(folder.join(ignored), b"x").unwrap_or_else(|_| panic!("ignored"));
        }
        let listed = list(&paths, id).unwrap_or_else(|_| panic!("list"));
        let names: Vec<_> = listed.screenshots.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["new.PNG", "mid.png", "old.png"]);
        assert_eq!(listed.screenshots[0].modified_ms, 30_000);
        assert!(!listed.truncated);
        cleanup(&paths);
    }

    #[test]
    fn names_that_could_leave_the_folder_or_confuse_windows_are_rejected() {
        let (paths, id, game) = fixture();
        std::fs::create_dir_all(game.join("screenshots")).unwrap_or_else(|_| panic!("folder"));
        std::fs::write(game.join("secret.png"), b"outside").unwrap_or_else(|_| panic!("outside"));
        std::fs::write(game.join("screenshots").join("ok.png"), b"image")
            .unwrap_or_else(|_| panic!("ok"));
        assert_eq!(read(&paths, id, "ok.png").ok(), Some(b"image".to_vec()));
        for bad in [
            "",
            ".",
            "..",
            "../secret.png",
            "..\\secret.png",
            "sub/ok.png",
            "C:\\x.png",
            "ok.png:stream",
            "ok.txt",
            "ok",
            ".png",
            "ok.png.",
            "ok.png ",
            "con.png",
            "NUL.png",
            "com1.png",
            "lpt9.png",
            "a|b.png",
            "a?.png",
            "tab\t.png",
        ] {
            assert!(read(&paths, id, bad).is_err(), "{bad:?}");
            assert!(locate(&paths, id, bad).is_err(), "{bad:?}");
        }
        assert!(read(&paths, id, "absent.png").is_err());
        assert_eq!(
            locate(&paths, id, "ok.png").ok(),
            Some(game.join("screenshots").join("ok.png"))
        );
        cleanup(&paths);
    }

    #[cfg(unix)]
    #[test]
    fn linked_screenshots_and_redirected_folders_are_rejected() {
        use std::os::unix::fs::symlink;
        let (paths, id, game) = fixture();
        let outside = game.join("outside");
        std::fs::create_dir_all(&outside).unwrap_or_else(|_| panic!("outside"));
        std::fs::write(outside.join("real.png"), b"secret").unwrap_or_else(|_| panic!("real"));
        let folder = game.join("screenshots");
        std::fs::create_dir_all(&folder).unwrap_or_else(|_| panic!("folder"));
        symlink(outside.join("real.png"), folder.join("link.png"))
            .unwrap_or_else(|_| panic!("link"));
        let listed = list(&paths, id).unwrap_or_else(|_| panic!("list"));
        assert!(listed.screenshots.is_empty());
        assert!(read(&paths, id, "link.png").is_err());
        assert!(locate(&paths, id, "link.png").is_err());
        std::fs::remove_file(folder.join("link.png")).unwrap_or_else(|_| panic!("unlink"));
        std::fs::remove_dir(&folder).unwrap_or_else(|_| panic!("rmdir"));
        symlink(&outside, &folder).unwrap_or_else(|_| panic!("redirect"));
        assert!(list(&paths, id).is_err());
        assert!(read(&paths, id, "real.png").is_err());
        assert!(locate(&paths, id, "real.png").is_err());
        cleanup(&paths);
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_in_place_of_the_screenshots_folder_is_rejected() {
        let (paths, id, game) = fixture();
        let outside = game.join("outside");
        std::fs::create_dir_all(&outside).unwrap_or_else(|_| panic!("outside"));
        std::fs::write(outside.join("real.png"), b"secret").unwrap_or_else(|_| panic!("real"));
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(game.join("screenshots"))
            .arg(&outside)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap_or_else(|_| panic!("mklink"));
        assert!(status.success());
        assert!(list(&paths, id).is_err());
        assert!(read(&paths, id, "real.png").is_err());
        assert!(locate(&paths, id, "real.png").is_err());
        cleanup(&paths);
    }

    #[test]
    fn png_images_decode_to_rgba_and_unusual_ones_are_normalized_or_refused() {
        let (paths, id, game) = fixture();
        let folder = game.join("screenshots");
        std::fs::create_dir_all(&folder).unwrap_or_else(|_| panic!("folder"));
        let rgb = encode(
            2,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            &[1, 2, 3, 4, 5, 6],
        );
        std::fs::write(folder.join("rgb.png"), rgb).unwrap_or_else(|_| panic!("rgb"));
        let decoded = read_rgba(&paths, id, "rgb.png").unwrap_or_else(|_| panic!("decode"));
        assert_eq!((decoded.width, decoded.height), (2, 1));
        assert_eq!(decoded.pixels, [1, 2, 3, 255, 4, 5, 6, 255]);
        let gray = encode(
            2,
            1,
            png::ColorType::GrayscaleAlpha,
            png::BitDepth::Eight,
            &[9, 100, 8, 200],
        );
        assert_eq!(
            decode_png(&gray).unwrap_or_else(|_| panic!("gray")).pixels,
            [9, 9, 9, 100, 8, 8, 8, 200]
        );
        let huge = encode(
            8200,
            8200,
            png::ColorType::Grayscale,
            png::BitDepth::One,
            &vec![0; 1025 * 8200],
        );
        assert!(decode_png(&huge).is_err());
        assert!(decode_png(b"not a png").is_err());
        std::fs::write(folder.join("fake.png"), b"not a png").unwrap_or_else(|_| panic!("fake"));
        assert!(read_rgba(&paths, id, "fake.png").is_err());
        cleanup(&paths);
    }
}
