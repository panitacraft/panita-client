use super::*;
use std::io::Cursor;
use zip::{
    CompressionMethod,
    write::{SimpleFileOptions, ZipWriter},
};

pub(crate) fn build(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, body) in entries {
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        if name.ends_with('/') {
            assert!(zip.add_directory(*name, options).is_ok());
        } else {
            assert!(zip.start_file(*name, options).is_ok());
            assert!(zip.write_all(body).is_ok());
        }
    }
    zip.finish().map(Cursor::into_inner).unwrap_or_default()
}

fn open(bytes: Vec<u8>, limits: Limits) -> Result<Opened<Cursor<Vec<u8>>>, CoreError> {
    preflight(Cursor::new(bytes), limits)
}

fn pack(bytes: Vec<u8>) -> Result<Opened<Cursor<Vec<u8>>>, CoreError> {
    open(bytes, Limits::PACK)
}

fn central(bytes: &[u8]) -> usize {
    bytes
        .windows(4)
        .position(|part| part == b"PK\x01\x02")
        .unwrap_or_default()
}

#[test]
fn a_clean_archive_lists_only_files_and_digests_their_exact_bytes() {
    let archive = build(&[
        ("config/", b""),
        ("options.txt", b"fov:90"),
        ("config/example.toml", b"key = 1\n"),
        ("kubejs/startup/script.js", b"// js"),
    ]);
    let Ok(mut opened) = pack(archive) else {
        panic!("clean archive");
    };
    let names: Vec<_> = opened.entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        names,
        [
            "options.txt",
            "config/example.toml",
            "kubejs/startup/script.js"
        ]
    );
    let mut budget = opened.budget();
    let entry = opened.entries[0].clone();
    let mut copy = Vec::new();
    let digest = opened.stream(
        &entry,
        &mut budget,
        &Cancellation::default(),
        Some(&mut copy),
    );
    assert_eq!(copy, b"fov:90");
    let digest = digest.unwrap_or_else(|_| panic!("digest"));
    assert_eq!(digest.size, 6);
    assert_eq!(
        digest.hashes.sha1.as_deref(),
        Some(format!("{:x}", Sha1::digest(b"fov:90")).as_str())
    );
    assert_eq!(
        digest.hashes.sha512.as_deref(),
        Some(format!("{:x}", Sha512::digest(b"fov:90")).as_str())
    );
    assert_eq!(budget, Limits::PACK.bytes - 6);
    assert!(pack(build(&[])).is_ok_and(|opened| opened.entries.is_empty()));
}

#[test]
fn hostile_names_and_content_types_are_rejected_before_any_write() {
    for name in [
        "../escape.txt",
        "config/../../escape.txt",
        "/absolute.txt",
        "C:/drive.txt",
        "config\\windows.txt",
        "config/CON.txt",
        "config/trailing.",
        "config/colon:stream",
        "saves/world/level.dat",
        "logs/latest.log",
        "instance.json",
        ".hidden/file.txt",
        "mods/extra.txt",
        "ResourcePacks/pack.txt",
        "shaderpacks/a.txt",
        "config/evil.jar",
        "config/Tool.EXE",
        "config/lib.dll",
        "config/run.bat",
        "config/run.cmd",
        "config/run.ps1",
        "config/run.sh",
        "config/lib.so",
        "config/lib.dylib",
        "config/setup.msi",
        "config/evil\u{202E}txt.exe",
        "config/zero\u{200B}width.txt",
    ] {
        assert!(pack(build(&[(name, b"data")])).is_err(), "{name}");
    }
}

#[test]
fn printable_unicode_names_are_accepted_and_fold_like_windows() {
    for name in ["config/ünicode.txt", "config/§3Fresh §bFlower Pots.txt"] {
        assert!(pack(build(&[(name, b"data")])).is_ok(), "{name}");
    }
    // Windows compares names across all of Unicode, so these are one file.
    assert!(pack(build(&[("config/É.txt", b"1"), ("config/é.txt", b"2")])).is_err());
    assert!(pack(build(&[("config/é.txt", b"1"), ("config/e.txt", b"2")])).is_ok());
}

#[test]
fn collisions_and_file_folder_conflicts_are_rejected() {
    assert!(pack(build(&[("Config/a.txt", b"1"), ("config/A.txt", b"2")])).is_err());
    assert!(pack(build(&[("config", b"1"), ("config/a.txt", b"2")])).is_err());
    assert!(pack(build(&[("config/a.txt", b"2"), ("config/a.txt/b", b"1")])).is_err());
    assert!(pack(build(&[("config/", b""), ("config", b"1")])).is_err());
}

#[test]
fn links_special_modes_encryption_and_zip64_are_rejected() {
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    assert!(
        zip.add_symlink("config/link", "../../outside", SimpleFileOptions::default())
            .is_ok()
    );
    let symlink = zip.finish().map(Cursor::into_inner).unwrap_or_default();
    assert!(pack(symlink).is_err());

    let mut encrypted = build(&[("config/a.txt", b"secret")]);
    let at = central(&encrypted) + 8;
    encrypted[at] |= 1;
    assert!(pack(encrypted).is_err());

    // More than 65,535 entries forces Zip64 end-of-directory records, which are refused.
    let many: Vec<(String, Vec<u8>)> = (0..70_000).map(|n| (format!("f{n}"), Vec::new())).collect();
    let refs: Vec<(&str, &[u8])> = many
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    let zip64 = build(&refs);
    assert!(zip64.windows(4).any(|part| part == b"PK"));
    assert!(pack(zip64).is_err());

    for broken in [
        Vec::new(),
        b"not a zip at all".to_vec(),
        build(&[("a.txt", b"hello world")])[..30].to_vec(),
    ] {
        assert!(pack(broken).is_err());
    }
}

#[test]
fn entry_and_size_limits_apply_to_the_directory() {
    let archive = build(&[
        ("a.txt", b"12345"),
        ("b.txt", b"12345"),
        ("c.txt", b"12345"),
    ]);
    assert!(
        open(
            archive.clone(),
            Limits {
                entries: 3,
                bytes: 15
            }
        )
        .is_ok()
    );
    assert!(
        open(
            archive.clone(),
            Limits {
                entries: 2,
                bytes: 15
            }
        )
        .is_err()
    );
    assert!(
        open(
            archive,
            Limits {
                entries: 3,
                bytes: 14
            }
        )
        .is_err()
    );
    let many: Vec<(String, Vec<u8>)> = (0..20_001).map(|n| (format!("f{n}"), Vec::new())).collect();
    let refs: Vec<(&str, &[u8])> = many
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    assert!(pack(build(&refs)).is_err());
}

#[test]
fn a_lying_directory_cannot_exceed_its_declared_size_or_budget() {
    let body = vec![b'0'; 10_000];
    let mut lying = build(&[("config/big.txt", &body)]);
    // The directory claims 100 bytes while the data inflates to 10,000.
    let at = central(&lying) + 24;
    lying[at..at + 4].copy_from_slice(&100u32.to_le_bytes());
    let result = pack(lying).and_then(|mut opened| {
        let entry = opened.entries[0].clone();
        let mut budget = opened.budget();
        let mut sink = Vec::new();
        opened.stream(
            &entry,
            &mut budget,
            &Cancellation::default(),
            Some(&mut sink),
        )
    });
    assert!(result.is_err());

    // The byte budget is shared by every entry, so the total is bounded while streaming.
    let archive = build(&[("a.txt", &[b'a'; 60]), ("b.txt", &[b'b'; 60])]);
    let Ok(mut opened) = open(
        archive,
        Limits {
            entries: 10,
            bytes: 200,
        },
    ) else {
        panic!("archive");
    };
    let mut budget = 100;
    let cancel = Cancellation::default();
    let (first, second) = (opened.entries[0].clone(), opened.entries[1].clone());
    assert!(opened.stream(&first, &mut budget, &cancel, None).is_ok());
    assert_eq!(budget, 40);
    assert!(opened.stream(&second, &mut budget, &cancel, None).is_err());
}

#[test]
fn corrupt_data_and_cancellation_fail_extraction() {
    let mut archive = build(&[("a.txt", b"data that will be corrupted in place")]);
    let mut stored = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    assert!(stored.start_file("a.txt", options).is_ok());
    assert!(stored.write_all(b"stored bytes to damage").is_ok());
    archive = stored.finish().map(Cursor::into_inner).unwrap_or(archive);
    let data = archive
        .windows(6)
        .position(|w| w == b"stored")
        .unwrap_or_default();
    archive[data] ^= 0xff;
    let Ok(mut opened) = pack(archive) else {
        panic!("preflight does not read data");
    };
    let entry = opened.entries[0].clone();
    let mut budget = opened.budget();
    assert!(
        opened
            .stream(&entry, &mut budget, &Cancellation::default(), None)
            .is_err()
    );

    let Ok(mut opened) = pack(build(&[("a.txt", b"fine")])) else {
        panic!("archive");
    };
    let cancel = Cancellation::default();
    cancel.cancel();
    let entry = opened.entries[0].clone();
    let mut budget = opened.budget();
    assert_eq!(
        opened
            .stream(&entry, &mut budget, &cancel, None)
            .err()
            .map(|e| e.code()),
        Some(crate::error::ErrorCode::OperationCancelled)
    );
}
