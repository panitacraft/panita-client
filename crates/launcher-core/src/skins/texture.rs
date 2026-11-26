//! Validation and canonical form of skin and cape images.
//!
//! Every image is decoded under strict limits and written back as a plain RGBA PNG, so
//! metadata chunks and odd encodings never reach the cache or the service. The identity
//! of a texture is the hash of its decoded pixels: the service re-encodes uploads, so a
//! hash of the file would not recognise the same skin when it comes back.
use std::io::Cursor;

use image::{ImageEncoder, ImageFormat, ImageReader, Limits, codecs::png::PngEncoder};
use sha2::{Digest, Sha256};

use crate::error::{CoreError, ErrorCode};

/// Largest file accepted as input. Real skins are a few kilobytes.
pub(crate) const MAX_INPUT_BYTES: usize = 1024 * 1024;
const MAX_SIDE: u32 = 1024;
const MAX_DECODE_BYTES: u64 = 64 * 1024 * 1024;

/// What an image is expected to be, which decides the sizes that qualify.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    /// A file the player supplies: only the sizes the service accepts.
    UploadSkin,
    /// A skin fetched from the texture host, which may be a larger, same-shaped image.
    RemoteSkin,
    Cape,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Texture {
    /// Hash of the decoded pixels and size, as 64 lowercase hexadecimal characters.
    pub hash: String,
    /// The canonical PNG that is stored and uploaded.
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

fn invalid() -> CoreError {
    CoreError::new(ErrorCode::InvalidSkin)
}

fn fits(kind: Kind, width: u32, height: u32) -> bool {
    match kind {
        Kind::UploadSkin => matches!((width, height), (64, 64) | (64, 32)),
        Kind::RemoteSkin => {
            width >= 64 && width.is_multiple_of(64) && (height == width || height * 2 == width)
        }
        // The standard cape is 64x32, the legacy one 22x17, and both scale in whole steps.
        Kind::Cape => {
            (width >= 64 && width.is_multiple_of(64) && height * 2 == width)
                || (width >= 22 && width.is_multiple_of(22) && height * 22 == width * 17)
        }
    }
}

/// Decode, check the size for `kind` and re-encode canonically.
pub(crate) fn prepare(bytes: &[u8], kind: Kind) -> Result<Texture, CoreError> {
    if bytes.is_empty() || bytes.len() > MAX_INPUT_BYTES {
        return Err(invalid());
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let rgba = reader.decode().map_err(|_| invalid())?.into_rgba8();
    let (width, height) = rgba.dimensions();
    if !fits(kind, width, height) {
        return Err(invalid());
    }
    let mut digest = Sha256::new();
    digest.update(width.to_be_bytes());
    digest.update(height.to_be_bytes());
    digest.update(rgba.as_raw());
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(
            rgba.as_raw(),
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|_| invalid())?;
    Ok(Texture {
        hash: format!("{:x}", digest.finalize()),
        png,
        width,
        height,
    })
}

#[cfg(test)]
pub(crate) fn encode_for_tests(width: u32, height: u32, shade: u8) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for index in 0..width * height {
        pixels.extend_from_slice(&[shade, (index % 251) as u8, (index / 251 % 251) as u8, 255]);
    }
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(&pixels, width, height, image::ExtendedColorType::Rgba8)
        .unwrap_or_else(|_| panic!("fixture encodes"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uploads_accept_only_the_sizes_the_service_accepts() {
        for (width, height) in [(64, 64), (64, 32)] {
            let texture = prepare(&encode_for_tests(width, height, 7), Kind::UploadSkin)
                .unwrap_or_else(|_| panic!("{width}x{height}"));
            assert_eq!((texture.width, texture.height), (width, height));
            assert_eq!(texture.hash.len(), 64);
        }
        for (width, height) in [(128, 128), (32, 32), (64, 48), (63, 64), (1, 1)] {
            assert_eq!(
                prepare(&encode_for_tests(width, height, 7), Kind::UploadSkin)
                    .err()
                    .map(|error| error.code()),
                Some(ErrorCode::InvalidSkin),
                "{width}x{height}"
            );
        }
    }

    #[test]
    fn fetched_skins_may_be_larger_in_the_same_shape() {
        for (width, height) in [(64, 64), (64, 32), (128, 128), (128, 64)] {
            assert!(prepare(&encode_for_tests(width, height, 1), Kind::RemoteSkin).is_ok());
        }
        assert!(prepare(&encode_for_tests(96, 96, 1), Kind::RemoteSkin).is_err());
        assert!(prepare(&encode_for_tests(64, 48, 1), Kind::RemoteSkin).is_err());
    }

    #[test]
    fn capes_may_be_standard_legacy_or_scaled() {
        for (width, height) in [(64, 32), (22, 17), (128, 64), (44, 34)] {
            assert!(
                prepare(&encode_for_tests(width, height, 1), Kind::Cape).is_ok(),
                "{width}x{height}"
            );
        }
        for (width, height) in [(64, 64), (23, 17), (64, 31)] {
            assert!(
                prepare(&encode_for_tests(width, height, 1), Kind::Cape).is_err(),
                "{width}x{height}"
            );
        }
    }

    #[test]
    fn identity_follows_the_pixels_not_the_file() {
        let first = prepare(&encode_for_tests(64, 64, 9), Kind::UploadSkin)
            .unwrap_or_else(|_| panic!("first"));
        // The canonical output decodes to the same pixels, so re-preparing it is stable.
        let again =
            prepare(&first.png, Kind::UploadSkin).unwrap_or_else(|_| panic!("canonical file"));
        assert_eq!(again.hash, first.hash);
        assert_eq!(again.png, first.png);
        let other = prepare(&encode_for_tests(64, 64, 10), Kind::UploadSkin)
            .unwrap_or_else(|_| panic!("other"));
        assert_ne!(other.hash, first.hash);
    }

    #[test]
    fn files_that_are_not_images_are_rejected_without_detail() {
        for bytes in [
            Vec::new(),
            b"not a png".to_vec(),
            b"\x89PNG\r\n\x1a\n truncated".to_vec(),
            vec![0; MAX_INPUT_BYTES + 1],
        ] {
            assert_eq!(
                prepare(&bytes, Kind::UploadSkin)
                    .err()
                    .map(|error| error.code()),
                Some(ErrorCode::InvalidSkin)
            );
        }
    }
}
