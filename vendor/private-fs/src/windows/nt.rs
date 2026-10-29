//! Single-component NT opens and handle-bound object metadata.
#![allow(unsafe_code)]
use super::{
    bindings::*,
    security::{self, Descriptor, Sid},
};
use crate::{Error, Result};
use std::{
    ffi::OsStr,
    io,
    mem::size_of,
    os::windows::{
        ffi::OsStrExt,
        io::{AsHandle, AsRawHandle, FromRawHandle, OwnedHandle},
    },
    ptr,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Identity {
    volume: u64,
    id: [u8; 16],
}

/// Resolve only the DOS drive alias before entering the filesystem. Applying
/// OBJ_DONT_REPARSE to \\??\\C:\\ can reject the normal Object Manager drive link.
/// Never relax that flag for filesystem paths or retry a rejected open without it.
pub(super) fn open_drive_root(drive: u8) -> Result<OwnedHandle> {
    if !drive.is_ascii_alphabetic() {
        return Err(Error::InvalidRelativePath);
    }
    let name = [u16::from(drive), u16::from(b':'), 0];
    let mut target = vec![0u16; 32768];
    // SAFETY: name is terminated and target covers the stated UTF-16 capacity.
    let length =
        unsafe { QueryDosDeviceW(name.as_ptr(), target.as_mut_ptr(), target.len() as u32) };
    if length == 0 {
        return Err(io::Error::last_os_error().into());
    }
    let target = target.get(..length as usize).ok_or(Error::PathRedirect)?;
    // The first terminated entry is the current mapping; subsequent entries are
    // historical mappings, not fallback destinations.
    let end = target
        .iter()
        .position(|unit| *unit == 0)
        .ok_or(Error::PathRedirect)?;
    let target = String::from_utf16(&target[..end]).map_err(|_| Error::PathRedirect)?;
    let anchor = device_root(&target)?;
    open(None, OsStr::new(&anchor), true, FILE_OPEN as u32, None)
}

fn device_root(target: &str) -> Result<String> {
    let device = target
        .strip_prefix(r"\Device\")
        .ok_or(Error::PathRedirect)?;
    // Accept one device object only. Reject SUBST, network mappings, chained DOS
    // aliases and directory suffixes so no filesystem ancestors are skipped.
    if device.is_empty()
        || matches!(device, "." | "..")
        || device.chars().any(|c| matches!(c, '\\' | '/' | ':' | '\0'))
    {
        return Err(Error::PathRedirect);
    }
    Ok(format!("{target}\\"))
}

#[cfg(test)]
mod drive_tests {
    use super::*;

    #[test]
    fn device_anchor_excludes_aliases_and_filesystem_suffixes() {
        assert_eq!(
            device_root(r"\Device\HarddiskVolume3").unwrap(),
            r"\Device\HarddiskVolume3\"
        );
        for target in [
            "",
            r"\??\C:\folder",
            r"\??\C:",
            r"\Device\",
            r"\Device\HarddiskVolume3\folder",
            r"\Device\HarddiskVolume3\",
            r"\Device\Mup\server\share",
            r"\Device\..",
            r"\Device\.",
            r"\Device\disk/path",
            r"\Device\disk:stream",
            "\\Device\\disk\0tail",
        ] {
            assert!(
                matches!(device_root(target), Err(Error::PathRedirect)),
                "{target:?}"
            );
        }
    }
}

pub(super) fn open(
    parent: Option<&OwnedHandle>,
    name: &OsStr,
    directory: bool,
    disposition: u32,
    user: Option<&Sid>,
) -> Result<OwnedHandle> {
    open_access(parent, name, directory, disposition, user, None)
}
fn open_access(
    parent: Option<&OwnedHandle>,
    name: &OsStr,
    directory: bool,
    disposition: u32,
    user: Option<&Sid>,
    desired_override: Option<u32>,
) -> Result<OwnedHandle> {
    let mut name: Vec<u16> = name.encode_wide().collect();
    if (name.is_empty() && desired_override.is_none())
        || name.contains(&0)
        || name.len() > (u16::MAX as usize) / 2
    {
        return Err(Error::InvalidRelativePath);
    }
    let bytes = (name.len() * 2) as u16;
    let mut string = UNICODE_STRING {
        Length: bytes,
        MaximumLength: bytes,
        Buffer: name.as_mut_ptr(),
    };
    // Pin the owner even when the token defaults to Administrators. Directories
    // inherit their DACL normally; only private files get a protected DACL.
    let mut security = user
        .map(|user| {
            if directory {
                Descriptor::inheriting_directory(user)
            } else {
                Descriptor::new(user)
            }
        })
        .transpose()?;
    let attributes = OBJECT_ATTRIBUTES {
        Length: size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.map_or(ptr::null_mut(), |p| p.as_raw_handle()),
        ObjectName: &mut string,
        // OBJ_INHERIT is deliberately absent at the original creation call.
        Attributes: (OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE) as u32,
        SecurityDescriptor: security
            .as_mut()
            .map_or(ptr::null_mut(), Descriptor::as_ptr),
        SecurityQualityOfService: ptr::null_mut(),
    };
    let creating = disposition != FILE_OPEN as u32;
    let desired = if directory {
        // Retained directory handles must not hold data or DELETE access:
        // Windows 10's internal rename-target open can conflict with it even
        // when our handles share read/write/delete. Directories use normal
        // inherited ACLs; mutations acquire short-lived handles.
        (FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE) as u32 | 0x20 // FILE_TRAVERSE
    } else if creating {
        FILE_ALL_ACCESS as u32
    } else {
        (FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE) as u32
            | if directory { 0 } else { FILE_READ_DATA as u32 }
    };
    let desired = desired_override.unwrap_or(desired);
    let mut handle = ptr::null_mut();
    let mut io_status = IO_STATUS_BLOCK::default();
    // Existing entries are opened type-neutral and inspected before use. New
    // entries must have the intended kind as part of their creation syscall.
    let options = FILE_OPEN_REPARSE_POINT as u32
        | if desired & SYNCHRONIZE as u32 != 0 {
            FILE_SYNCHRONOUS_IO_NONALERT as u32
        } else {
            0
        }
        | if creating {
            if directory {
                FILE_DIRECTORY_FILE as u32
            } else {
                FILE_NON_DIRECTORY_FILE as u32
            }
        } else {
            0
        };
    // SAFETY: all names, outputs, and optional absolute security descriptor are
    // live and correctly aligned until this synchronous call completes.
    let status = unsafe {
        NtCreateFile(
            &mut handle,
            desired,
            &attributes,
            &mut io_status,
            ptr::null(),
            FILE_ATTRIBUTE_NORMAL as u32,
            (FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE) as u32,
            disposition,
            options,
            ptr::null(),
            0,
        )
    };
    if status < 0 {
        if status == STATUS_REPARSE_POINT_ENCOUNTERED || status == STATUS_STOPPED_ON_SYMLINK {
            return Err(Error::PathRedirect);
        }
        // SAFETY: scalar NTSTATUS conversion does not dereference pointers.
        return Err(
            io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32).into(),
        );
    }
    // SAFETY: successful NtCreateFile transferred a unique non-inheritable handle.
    let handle = unsafe { OwnedHandle::from_raw_handle(handle) };
    if desired & FILE_READ_ATTRIBUTES as u32 != 0 {
        kind(&handle, directory)?;
    }
    Ok(handle)
}
fn info<T: Default>(handle: impl AsHandle, class: FILE_INFO_BY_HANDLE_CLASS) -> Result<T> {
    let mut value = T::default();
    // SAFETY: internal callers match the class to its exact output structure.
    security::bool_result(unsafe {
        GetFileInformationByHandleEx(
            handle.as_handle().as_raw_handle(),
            class,
            (&mut value as *mut T).cast(),
            size_of::<T>() as u32,
        )
    })?;
    Ok(value)
}
pub(super) fn kind(handle: impl AsHandle, directory: bool) -> Result<()> {
    // SAFETY: live borrowed handle; GetFileType has no output pointer.
    if unsafe { GetFileType(handle.as_handle().as_raw_handle()) } != FILE_TYPE_DISK as u32 {
        return Err(Error::WrongObjectKind);
    }
    let attributes: FILE_ATTRIBUTE_TAG_INFO = info(handle.as_handle(), FileAttributeTagInfo)?;
    if attributes.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT as u32 != 0 {
        return Err(Error::PathRedirect);
    }
    if (attributes.FileAttributes & FILE_ATTRIBUTE_DIRECTORY as u32 != 0) != directory {
        return Err(Error::WrongObjectKind);
    }
    Ok(())
}
pub(super) fn volume(handle: impl AsHandle) -> Result<()> {
    let mut flags = 0;
    // SAFETY: optional buffers are null/zero; flags is a writable u32 output.
    security::bool_result(unsafe {
        GetVolumeInformationByHandleW(
            handle.as_handle().as_raw_handle(),
            ptr::null_mut(),
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut flags,
            ptr::null_mut(),
            0,
        )
    })?;
    volume_flags(flags)
}
pub(super) fn volume_flags(flags: u32) -> Result<()> {
    if flags & FILE_PERSISTENT_ACLS as u32 == 0 {
        return Err(Error::UnsupportedFilesystem {
            capability: "persistent Windows ACLs",
        });
    }
    Ok(())
}
pub(super) fn metadata(
    handle: impl AsHandle,
    directory: bool,
    expected: Option<Identity>,
) -> Result<Identity> {
    kind(handle.as_handle(), directory)?;
    volume(handle.as_handle())?;
    let value: FILE_ID_INFO = info(handle.as_handle(), FileIdInfo)?;
    let id = Identity {
        volume: value.VolumeSerialNumber,
        id: value.FileId.Identifier,
    };
    if expected.is_some_and(|expected| expected != id) {
        return Err(Error::IdentityChanged);
    }
    let standard: FILE_STANDARD_INFO = info(handle.as_handle(), FileStandardInfo)?;
    if standard.Directory != directory || standard.DeletePending {
        return Err(Error::WrongObjectKind);
    }
    if !directory && standard.NumberOfLinks != 1 {
        return Err(Error::MultipleHardLinks {
            links: u64::from(standard.NumberOfLinks),
        });
    }
    Ok(id)
}
pub(super) fn validate(
    handle: impl AsHandle,
    directory: bool,
    expected: Option<Identity>,
    user: &Sid,
) -> Result<Identity> {
    let id = metadata(handle.as_handle(), directory, expected)?;
    if directory {
        security::verify_owner(handle, user)?;
    } else {
        security::verify(handle, user)?;
    }
    Ok(id)
}

pub(super) fn flush(handle: impl AsHandle) -> Result<()> {
    // SAFETY: live borrowed handle; the call is synchronous and has no outputs.
    security::bool_result(unsafe { FlushFileBuffers(handle.as_handle().as_raw_handle()) })
}

pub(super) fn directory_for_sync(parent: &OwnedHandle) -> Result<OwnedHandle> {
    open_access(
        Some(parent),
        OsStr::new(""),
        true,
        FILE_OPEN as u32,
        None,
        Some((FILE_READ_ATTRIBUTES | FILE_WRITE_DATA | READ_CONTROL | SYNCHRONIZE) as u32),
    )
}

pub(super) fn rename(file: impl AsHandle, parent: &OwnedHandle, target: &OsStr) -> Result<()> {
    rename_impl(file, parent, target, false)
}
#[cfg(test)]
pub(super) fn rename_classic(
    file: impl AsHandle,
    parent: &OwnedHandle,
    target: &OsStr,
) -> Result<()> {
    rename_impl(file, parent, target, true)
}
fn rename_impl(
    file: impl AsHandle,
    parent: &OwnedHandle,
    target: &OsStr,
    force_classic: bool,
) -> Result<()> {
    let name: Vec<u16> = target.encode_wide().collect();
    let offset = std::mem::offset_of!(FILE_RENAME_INFORMATION, FileName);
    let bytes = (offset + name.len() * 2).max(size_of::<FILE_RENAME_INFORMATION>());
    let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
    let rename = storage.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
    // SAFETY: pointer-aligned initialized allocation covers the fixed structure
    // and UTF-16 tail. RootDirectory pins the destination; source is the live
    // file handle. No complete pathname or mutable source name is reopened.
    unsafe {
        (*rename).Anonymous.Flags =
            (FILE_RENAME_REPLACE_IF_EXISTS | FILE_RENAME_POSIX_SEMANTICS) as u32;
        (*rename).RootDirectory = parent.as_raw_handle();
        (*rename).FileNameLength = (name.len() * 2) as u32;
        ptr::copy_nonoverlapping(
            name.as_ptr(),
            storage.as_mut_ptr().cast::<u8>().add(offset).cast(),
            name.len(),
        );
        let mut status = IO_STATUS_BLOCK::default();
        let result = if force_classic {
            // Test-only callers simulate an unavailable extended class; execute
            // the real classic syscall with the same source and parent handles.
            STATUS_INVALID_INFO_CLASS
        } else {
            NtSetInformationFile(
                file.as_handle().as_raw_handle(),
                &mut status,
                rename.cast(),
                bytes as u32,
                FileRenameInformationEx,
            )
        };
        if matches!(
            result,
            STATUS_INVALID_INFO_CLASS | STATUS_NOT_SUPPORTED | STATUS_INVALID_PARAMETER
        ) {
            // Older kernels/filesystems retain classic atomic rename semantics,
            // but can refuse open destinations. Never emulate replacement by unlink.
            (*rename).Anonymous.Flags = 0;
            (*rename).Anonymous.ReplaceIfExists = true;
            return nt_result(NtSetInformationFile(
                file.as_handle().as_raw_handle(),
                &mut status,
                rename.cast(),
                bytes as u32,
                FileRenameInformation,
            ));
        }
        nt_result(result)
    }
}
pub(super) fn delete(file: impl AsHandle) -> Result<()> {
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    // SAFETY: live DELETE-capable file handle and matching initialized payload.
    // The OS marks this exact opened object for removal when its handles close.
    security::bool_result(unsafe {
        SetFileInformationByHandle(
            file.as_handle().as_raw_handle(),
            FileDispositionInfo,
            (&disposition as *const FILE_DISPOSITION_INFO).cast(),
            size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    })
}

fn nt_result(status: NTSTATUS) -> Result<()> {
    if status >= 0 {
        return Ok(());
    }
    // SAFETY: scalar status conversion only.
    Err(io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(status) } as i32).into())
}
pub(super) fn identity_matches(handle: impl AsHandle, expected: Identity) -> Result<()> {
    let value: FILE_ID_INFO = info(handle, FileIdInfo)?;
    if (Identity {
        volume: value.VolumeSerialNumber,
        id: value.FileId.Identifier,
    }) != expected
    {
        return Err(Error::IdentityChanged);
    }
    Ok(())
}
pub(super) fn security_handle(handle: impl AsHandle) -> Result<OwnedHandle> {
    let owned = handle.as_handle().try_clone_to_owned()?;
    open_access(
        Some(&owned),
        OsStr::new(""),
        false,
        FILE_OPEN as u32,
        None,
        Some((READ_CONTROL | WRITE_DAC) as u32),
    )
}

/// Change visibility through the retained object, never through its pathname.
pub(super) fn hide(handle: impl AsHandle, directory: bool, id: Identity, user: &Sid) -> Result<()> {
    const WRITE_ATTRIBUTES: u32 = 0x100;
    const HIDDEN: u32 = 2;
    const BASIC_INFO: FILE_INFO_BY_HANDLE_CLASS = 0;
    validate(handle.as_handle(), directory, Some(id), user)?;
    let owned = handle.as_handle().try_clone_to_owned()?;
    let writable = open_access(
        Some(&owned),
        OsStr::new(""),
        directory,
        FILE_OPEN as u32,
        None,
        Some((FILE_READ_ATTRIBUTES | READ_CONTROL) as u32 | WRITE_ATTRIBUTES),
    )?;
    validate(&writable, directory, Some(id), user)?;
    #[repr(C)]
    #[derive(Default)]
    struct BasicInfo {
        creation: i64,
        access: i64,
        write: i64,
        change: i64,
        attributes: u32,
    }
    let mut basic: BasicInfo = info(&writable, BASIC_INFO)?;
    if basic.attributes & HIDDEN == 0 {
        // Zero timestamps preserve the existing values, including concurrent writes.
        basic = BasicInfo {
            attributes: (basic.attributes & !(FILE_ATTRIBUTE_NORMAL as u32)) | HIDDEN,
            ..BasicInfo::default()
        };
        // SAFETY: writable is verified and basic has the FILE_BASIC_INFO layout.
        security::bool_result(unsafe {
            SetFileInformationByHandle(
                writable.as_raw_handle(),
                BASIC_INFO,
                (&basic as *const BasicInfo).cast(),
                size_of::<BasicInfo>() as u32,
            )
        })?;
    }
    validate(&writable, directory, Some(id), user).map(|_| ())
}
