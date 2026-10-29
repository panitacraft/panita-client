use crate::{Error, Result, path::RelativePath};
use rustix::fs::{self, AtFlags, FileType, Mode, OFlags};
use std::{
    ffi::OsStr,
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    os::fd::{AsFd, BorrowedFd, OwnedFd},
    path::{Component, Path},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Identity {
    dev: u64,
    ino: u64,
}
fn identity(s: &fs::Stat) -> Identity {
    Identity {
        #[cfg(target_os = "linux")]
        dev: s.st_dev,
        #[cfg(target_os = "macos")]
        dev: u64::from(s.st_dev as u32),
        ino: s.st_ino,
    }
}
fn creation_parent(fd: impl AsFd) -> Result<()> {
    let stat = fs::fstat(fd.as_fd())?;
    creation_parent_stat(&stat)?;
    #[cfg(target_os = "macos")]
    crate::macos::creation_parent(fd)?;
    Ok(())
}
fn creation_parent_stat(stat: &fs::Stat) -> Result<()> {
    // mkdirat cannot return the created object's FD. A trusted owner and either
    // exclusive namespace writes or sticky protection bind the first observation
    // to our creation against other principals. Same-UID/root actors are trusted.
    if (stat.st_uid != rustix::process::geteuid().as_raw() && stat.st_uid != 0)
        || (stat.st_mode & 0o022 != 0 && stat.st_mode & 0o1000 == 0)
    {
        return Err(Error::UntrustedCreationParent);
    }
    Ok(())
}
#[derive(Clone, Copy, Debug)]
enum Resolver {
    Auto,
    #[cfg(all(test, target_os = "linux"))]
    Unavailable,
    #[cfg(target_os = "linux")]
    Walk,
    #[cfg(target_os = "linux")]
    OpenAt2,
}

#[derive(Debug)]
/// A verified directory capability with an explicit private/ordinary file policy.
pub struct PrivateDir {
    fd: OwnedFd,
    id: Identity,
    resolver: Resolver,
    private_files: bool,
}
#[derive(Debug)]
/// A verified regular file. Native-handle operations are the caller's responsibility.
pub struct PrivateFile {
    file: File,
    id: Identity,
    links: u64,
    private: bool,
}

fn permission_error(expected: impl Into<u32>, actual: impl Into<u32>) -> Error {
    Error::InsecurePermissions {
        expected: expected.into(),
        actual: actual.into(),
    }
}
fn link_count(value: impl Into<u64>) -> u64 {
    value.into()
}
fn link_error(links: impl Into<u64>) -> Error {
    Error::MultipleHardLinks {
        links: links.into(),
    }
}
fn validate(fd: impl AsFd, directory: bool, expected_id: Option<Identity>) -> Result<Identity> {
    validate_links(fd, directory, expected_id, 1)
}

/// Data access follows the kernel's access checks, retaining object and link
/// identity without imposing private ownership or exact modes.
#[cfg(target_os = "linux")]
fn validate_data_stat(
    s: &fs::Stat,
    directory: bool,
    expected: Option<Identity>,
    links: u64,
) -> Result<Identity> {
    if expected.is_some_and(|id| id != identity(s)) {
        return Err(Error::IdentityChanged);
    }
    let kind = if directory {
        FileType::Directory
    } else {
        FileType::RegularFile
    };
    if FileType::from_raw_mode(s.st_mode) != kind {
        return Err(Error::WrongObjectKind);
    }
    if !directory && link_count(s.st_nlink) != links {
        if links == 0 {
            return Err(Error::UnexpectedLinkCount {
                expected: 0,
                actual: link_count(s.st_nlink),
            });
        }
        return Err(link_error(s.st_nlink));
    }
    Ok(identity(s))
}

fn validate_directory(fd: impl AsFd, expected: Option<Identity>) -> Result<Identity> {
    #[cfg(target_os = "linux")]
    {
        validate_data_stat(&fs::fstat(fd)?, true, expected, 1)
    }
    #[cfg(not(target_os = "linux"))]
    {
        validate(fd, true, expected)
    }
}

fn validate_file(
    fd: impl AsFd,
    expected: Option<Identity>,
    links: u64,
    private: bool,
) -> Result<Identity> {
    #[cfg(target_os = "linux")]
    if !private {
        return validate_data_stat(&fs::fstat(fd)?, false, expected, links);
    }
    let _ = private;
    validate_links(fd, false, expected, links)
}
fn validate_links(
    fd: impl AsFd,
    directory: bool,
    expected_id: Option<Identity>,
    links: u64,
) -> Result<Identity> {
    #[cfg(target_os = "macos")]
    crate::macos::ownership(fd.as_fd())?;
    let s = fs::fstat(fd.as_fd())?;
    let id = validate_stat_links(&s, directory, expected_id, links)?;
    #[cfg(target_os = "macos")]
    crate::macos::verify_acl(fd)?;
    Ok(id)
}
fn validate_stat(s: &fs::Stat, directory: bool, expected_id: Option<Identity>) -> Result<Identity> {
    validate_stat_links(s, directory, expected_id, 1)
}
fn validate_stat_links(
    s: &fs::Stat,
    directory: bool,
    expected_id: Option<Identity>,
    links: u64,
) -> Result<Identity> {
    if expected_id.is_some_and(|id| id != identity(s)) {
        return Err(Error::IdentityChanged);
    }
    let kind = FileType::from_raw_mode(s.st_mode);
    if kind
        != if directory {
            FileType::Directory
        } else {
            FileType::RegularFile
        }
    {
        return Err(Error::WrongObjectKind);
    }
    let expected = rustix::process::geteuid().as_raw();
    if s.st_uid != expected {
        return Err(Error::WrongOwner {
            expected,
            actual: s.st_uid,
        });
    }
    let expected = if directory { 0o700 } else { 0o600 };
    let actual = s.st_mode & 0o7777;
    if actual != expected {
        return Err(permission_error(expected, actual));
    }
    if !directory && link_count(s.st_nlink) != links {
        if links == 0 {
            return Err(Error::UnexpectedLinkCount {
                expected: 0,
                actual: link_count(s.st_nlink),
            });
        }
        return Err(link_error(s.st_nlink));
    }
    Ok(identity(s))
}
#[cfg(target_os = "linux")]
fn open_fast(
    parent: impl AsFd,
    path: &OsStr,
    flags: OFlags,
    mode: Mode,
    resolver: Resolver,
) -> Result<Option<OwnedFd>> {
    if matches!(resolver, Resolver::Walk) {
        return Ok(None);
    }
    #[cfg(test)]
    let unavailable = matches!(resolver, Resolver::Unavailable);
    #[cfg(not(test))]
    let unavailable = false;
    let result = if unavailable {
        Err(rustix::io::Errno::NOSYS)
    } else {
        fs::openat2(
            parent,
            path,
            flags | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
            mode,
            fs::ResolveFlags::BENEATH
                | fs::ResolveFlags::NO_SYMLINKS
                | fs::ResolveFlags::NO_MAGICLINKS,
        )
    };
    match result {
        Ok(fd) => Ok(Some(fd)),
        Err(e) if e == rustix::io::Errno::NOSYS => {
            if matches!(resolver, Resolver::OpenAt2) {
                return Err(e.into());
            }
            Ok(None)
        }
        Err(e) => Err(e.into()),
    }
}
fn open_component(
    parent: impl AsFd,
    name: &OsStr,
    flags: OFlags,
    mode: Mode,
    resolver: Resolver,
) -> Result<OwnedFd> {
    let flags = flags | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK;
    #[cfg(target_os = "linux")]
    if let Some(fd) = open_fast(parent.as_fd(), name, flags, mode, resolver)? {
        return Ok(fd);
    }
    #[cfg(not(target_os = "linux"))]
    let _ = resolver;
    // Check redirects through the parent descriptor when O_DIRECTORY reports ENOTDIR.
    match fs::openat(parent.as_fd(), name, flags, mode) {
        Ok(fd) => Ok(fd),
        Err(e) => {
            if e == rustix::io::Errno::NOTDIR
                && fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
                    .is_ok_and(|s| FileType::from_raw_mode(s.st_mode) == FileType::Symlink)
            {
                Err(Error::PathRedirect)
            } else {
                Err(e.into())
            }
        }
    }
}
fn normalize_created(fd: impl AsFd, directory: bool, id: Identity) -> Result<()> {
    #[cfg(target_os = "macos")]
    crate::macos::verify(fd.as_fd())?;
    let s = fs::fstat(fd.as_fd())?;
    let expected = if directory { 0o700 } else { 0o600 };
    let actual = s.st_mode & 0o7777;
    if actual == expected {
        return validate(fd, directory, Some(id)).map(|_| ());
    }
    if actual & !expected != 0 {
        return Err(permission_error(expected, actual));
    }
    // Validate every property except the narrower permission bits before widening.
    let mut normalized = s;
    normalized.st_mode = (s.st_mode & !0o7777) | expected;
    validate_stat(&normalized, directory, Some(id))?;
    fs::fchmod(fd.as_fd(), Mode::from_raw_mode(expected))?;
    validate(fd, directory, Some(id)).map(|_| ())
}
fn harden(fd: impl AsFd, directory: bool, id: Identity) -> Result<()> {
    #[cfg(target_os = "macos")]
    crate::macos::ownership(fd.as_fd())?;
    let mut stat = fs::fstat(fd.as_fd())?;
    let mode = if directory { 0o700 } else { 0o600 };
    stat.st_mode = (stat.st_mode & !0o7777) | mode;
    validate_stat(&stat, directory, Some(id))?;
    fs::fchmod(fd.as_fd(), Mode::from_raw_mode(mode))?;
    #[cfg(target_os = "macos")]
    crate::macos::clear(fd.as_fd())?;
    validate(fd, directory, Some(id)).map(|_| ())
}
impl PrivateDir {
    /// Validate Linux directories without permission changes; harden macOS directories.
    pub fn harden(&self) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            self.check_private()
        }
        #[cfg(not(target_os = "linux"))]
        harden(&self.fd, true, self.id)
    }
    /// Open and verify a root. Every ancestor is resolved without following links.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::acquire(path.as_ref(), false, Resolver::Auto)
    }
    /// Create the final root if absent. Existing ancestors are never hardened.
    pub fn ensure(path: impl AsRef<Path>) -> Result<Self> {
        Self::acquire(path.as_ref(), true, Resolver::Auto)
    }
    fn acquire(path: &Path, create: bool, resolver: Resolver) -> Result<Self> {
        Self::acquire_policy(path, create, resolver, true)
    }
    #[cfg(target_os = "linux")]
    pub fn open_data(path: impl AsRef<Path>) -> Result<Self> {
        Self::acquire_policy(path.as_ref(), false, Resolver::Auto, false)
    }
    #[cfg(target_os = "linux")]
    pub fn ensure_data(path: impl AsRef<Path>) -> Result<Self> {
        Self::acquire_policy(path.as_ref(), true, Resolver::Auto, false)
    }
    #[cfg(target_os = "linux")]
    pub fn verify_private_namespace(&self) -> Result<()> {
        validate_directory(&self.fd, Some(self.id))?;
        creation_parent(&self.fd)?;
        let stat = fs::fstat(&self.fd)?;
        let mode = stat.st_mode & 0o7777;
        if mode & 0o022 != 0 {
            return Err(permission_error(mode & !0o022, mode));
        }
        Ok(())
    }
    fn acquire_policy(
        path: &Path,
        create: bool,
        resolver: Resolver,
        private_files: bool,
    ) -> Result<Self> {
        crate::path::validate_component_spelling(path)?;
        if !path.is_absolute() {
            RelativePath::new(path)?;
        }
        #[cfg(all(target_os = "linux", feature = "force-walk"))]
        let resolver = if matches!(resolver, Resolver::Auto) {
            Resolver::Walk
        } else {
            resolver
        };
        #[cfg(target_os = "macos")]
        let canonical = canonical_alias(path);
        #[cfg(target_os = "macos")]
        let path = canonical.as_path();
        let mut names = Vec::new();
        for c in path.components() {
            match c {
                Component::Normal(n) => names.push(n),
                Component::RootDir => (),
                _ => return Err(Error::InvalidRelativePath),
            }
        }
        let leaf = names.pop().ok_or(Error::InvalidRelativePath)?;
        let mut parent = fs::open(
            if path.is_absolute() { "/" } else { "." },
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        #[cfg(target_os = "linux")]
        let resolver = if matches!(resolver, Resolver::Auto) {
            if open_fast(
                &parent,
                OsStr::new("."),
                OFlags::RDONLY | OFlags::DIRECTORY,
                Mode::empty(),
                resolver,
            )?
            .is_some()
            {
                Resolver::OpenAt2
            } else {
                Resolver::Walk
            }
        } else {
            resolver
        };
        #[cfg(target_os = "linux")]
        let fast = if names.is_empty() {
            None
        } else {
            let relative: std::path::PathBuf = names.iter().collect();
            open_fast(
                &parent,
                relative.as_os_str(),
                OFlags::RDONLY | OFlags::DIRECTORY,
                Mode::empty(),
                resolver,
            )?
        };
        #[cfg(not(target_os = "linux"))]
        let fast: Option<OwnedFd> = None;
        if let Some(fd) = fast {
            parent = fd;
        } else {
            for n in names {
                parent = open_component(
                    &parent,
                    n,
                    OFlags::RDONLY | OFlags::DIRECTORY,
                    Mode::empty(),
                    resolver,
                )?;
            }
        }
        Self::directory_at(&parent, leaf, create, false, resolver, private_files)
    }
    fn directory_at(
        parent: impl AsFd,
        name: &OsStr,
        create: bool,
        exclusive: bool,
        resolver: Resolver,
        private_files: bool,
    ) -> Result<Self> {
        if create {
            if !exclusive {
                match open_component(
                    parent.as_fd(),
                    name,
                    OFlags::RDONLY | OFlags::DIRECTORY,
                    Mode::empty(),
                    resolver,
                ) {
                    Ok(fd) => {
                        let id = validate_directory(&fd, None)?;
                        return Ok(Self {
                            fd,
                            id,
                            resolver,
                            private_files,
                        });
                    }
                    Err(Error::Io(e)) if e.kind() == io::ErrorKind::NotFound => (),
                    Err(e) => return Err(e),
                }
            }
            #[cfg(not(target_os = "linux"))]
            creation_parent(parent.as_fd())?;
        }
        let created = if create {
            match fs::mkdirat(
                parent.as_fd(),
                name,
                Mode::from_raw_mode(if private_files { 0o700 } else { 0o777 }),
            ) {
                Ok(()) => true,
                Err(e) if e == rustix::io::Errno::EXIST && !exclusive => false,
                Err(e) => return Err(e.into()),
            }
        } else {
            false
        };
        // mkdirat returns no descriptor. Bind its observed identity to the opened
        // object before returning; never chmod or unlink a possibly replaced name.
        let observed = if created {
            Some(identity(&fs::statat(
                parent.as_fd(),
                name,
                AtFlags::SYMLINK_NOFOLLOW,
            )?))
        } else {
            None
        };
        let fd = open_component(
            parent,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY,
            Mode::empty(),
            resolver,
        )?;
        if let Some(id) = observed {
            if private_files {
                normalize_created(&fd, true, id)?;
            }
        }
        let id = validate_directory(&fd, observed)?;
        Ok(Self {
            fd,
            id,
            resolver,
            private_files,
        })
    }
    fn parent<'a>(&self, path: &'a Path) -> Result<(OwnedFd, &'a OsStr)> {
        validate_directory(&self.fd, Some(self.id))?;
        let mut p = RelativePath::new(path)?.0;
        let leaf = p.pop().ok_or(Error::InvalidRelativePath)?;
        let mut parent = rustix::io::fcntl_dupfd_cloexec(&self.fd, 0)?;
        for name in p {
            parent = open_component(
                &parent,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY,
                Mode::empty(),
                self.resolver,
            )?;
            validate_directory(&parent, None)?;
        }
        Ok((parent, leaf))
    }
    /// Open an existing regular file for reading, without metadata mutation.
    pub fn open_file(&self, path: impl AsRef<Path>) -> Result<PrivateFile> {
        self.open_file_policy(path.as_ref(), self.private_files)
    }
    #[cfg(target_os = "linux")]
    pub fn open_data_file(&self, path: impl AsRef<Path>) -> Result<PrivateFile> {
        self.open_file_policy(path.as_ref(), false)
    }
    fn open_file_policy(&self, path: &Path, private: bool) -> Result<PrivateFile> {
        let (parent, leaf) = self.parent(path.as_ref())?;
        let fd = open_component(&parent, leaf, OFlags::RDONLY, Mode::empty(), self.resolver)?;
        let id = validate_file(&fd, None, 1, private)?;
        Ok(PrivateFile {
            file: fd.into(),
            id,
            links: 1,
            private,
        })
    }
    /// Exclusively create a readable and writable file with initial mode 0600.
    pub fn create(&self, path: impl AsRef<Path>) -> Result<PrivateFile> {
        let (parent, leaf) = self.parent(path.as_ref())?;
        #[cfg(target_os = "macos")]
        crate::macos::creation_parent(&parent)?;
        let fd = open_component(
            &parent,
            leaf,
            OFlags::RDWR | OFlags::CREATE | OFlags::EXCL,
            Mode::from_raw_mode(if self.private_files { 0o600 } else { 0o666 }),
            self.resolver,
        )?;
        let s = fs::fstat(&fd)?;
        let id = identity(&s);
        if self.private_files {
            normalize_created(&fd, false, id)?;
        }
        validate_file(&fd, Some(id), 1, self.private_files)?;
        Ok(PrivateFile {
            file: fd.into(),
            id,
            links: 1,
            private: self.private_files,
        })
    }
    /// Open an existing private directory relative to this retained capability.
    pub fn open_dir(&self, path: impl AsRef<Path>) -> Result<Self> {
        let (parent, leaf) = self.parent(path.as_ref())?;
        Self::directory_at(
            parent,
            leaf,
            false,
            false,
            self.resolver,
            self.private_files,
        )
    }
    /// Exclusively create the final directory. Private parents must already exist.
    pub fn create_dir(&self, path: impl AsRef<Path>) -> Result<Self> {
        let (parent, leaf) = self.parent(path.as_ref())?;
        Self::directory_at(parent, leaf, true, true, self.resolver, self.private_files)
    }
    /// Create or verify the final directory, without repairing existing entries.
    pub fn ensure_dir(&self, path: impl AsRef<Path>) -> Result<Self> {
        let (parent, leaf) = self.parent(path.as_ref())?;
        Self::directory_at(parent, leaf, true, false, self.resolver, self.private_files)
    }
}
impl PrivateFile {
    /// Restore private-file access; ordinary data files are only revalidated.
    pub fn harden(&self) -> Result<()> {
        if !self.private {
            return self.verify();
        }
        harden(&self.file, false, self.id)
    }
    /// Recheck mutable security metadata on the retained descriptor.
    pub fn verify(&self) -> Result<()> {
        validate_file(&self.file, Some(self.id), self.links, self.private).map(|_| ())
    }
    /// Sync file data with security revalidation; does not sync a parent directory.
    pub fn sync_data(&self) -> Result<()> {
        self.verify()?;
        self.file.sync_data()?;
        self.verify()
    }
    /// Sync file contents and metadata; use durable atomic writes for name persistence.
    /// This is the standard file sync, not macOS atomic `Full`'s stronger flush.
    pub fn sync_all(&self) -> Result<()> {
        self.verify()?;
        self.file.sync_all()?;
        self.verify()
    }
    fn check_io(&self) -> io::Result<()> {
        self.verify().map_err(io::Error::other)
    }
}
impl Read for PrivateFile {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        self.check_io()?;
        let n = self.file.read(b)?;
        self.check_io()?;
        Ok(n)
    }
}
impl Write for PrivateFile {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.check_io()?;
        let n = self.file.write(b)?;
        self.check_io()?;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.check_io()?;
        self.file.flush()?;
        self.check_io()
    }
}
impl Seek for PrivateFile {
    fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
        self.check_io()?;
        self.file.seek(p)
    }
}
impl AsFd for PrivateFile {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.file.as_fd()
    }
}
impl AsFd for PrivateDir {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests;

#[cfg(target_os = "macos")]
fn canonical_alias(path: &Path) -> std::path::PathBuf {
    for alias in ["/var", "/tmp", "/etc"] {
        if let Ok(suffix) = path.strip_prefix(alias) {
            return Path::new("/private")
                .join(alias.trim_start_matches('/'))
                .join(suffix);
        }
    }
    path.to_path_buf()
}

pub(crate) fn verify_existing(file: &File) -> Result<()> {
    validate(file, false, None).map(|_| ())
}
#[cfg(target_os = "linux")]
pub(crate) fn verify_data(file: &File) -> Result<()> {
    validate_file(file, None, 1, false).map(|_| ())
}
pub(crate) fn harden_existing(file: &File) -> Result<()> {
    let id = identity(&fs::fstat(file)?);
    harden(file, false, id)
}
impl PrivateDir {
    pub(crate) fn atomic_parent(&self, path: &Path) -> Result<(Self, std::ffi::OsString)> {
        let (fd, leaf) = self.parent(path)?;
        let id = validate_directory(&fd, None)?;
        Ok((
            Self {
                fd,
                id,
                resolver: self.resolver,
                private_files: self.private_files,
            },
            leaf.to_os_string(),
        ))
    }
    pub(crate) fn check_private(&self) -> Result<()> {
        validate_directory(&self.fd, Some(self.id)).map(|_| ())
    }
    pub(crate) fn anonymous_temp(&self) -> Result<Option<PrivateFile>> {
        self.check_private()?;
        #[cfg(target_os = "linux")]
        {
            let fd = match open_component(
                &self.fd,
                OsStr::new("."),
                OFlags::RDWR | OFlags::TMPFILE,
                Mode::from_raw_mode(if self.private_files { 0o600 } else { 0o666 }),
                self.resolver,
            ) {
                Ok(fd) => fd,
                Err(Error::Io(e))
                    if e.raw_os_error().is_some_and(|code| {
                        matches!(
                            rustix::io::Errno::from_raw_os_error(code),
                            rustix::io::Errno::OPNOTSUPP
                                | rustix::io::Errno::ISDIR
                                | rustix::io::Errno::INVAL
                        )
                    }) =>
                {
                    return Ok(None);
                }
                Err(e) => return Err(e),
            };
            let mut stat = fs::fstat(&fd)?;
            let id = identity(&stat);
            let actual = stat.st_mode & 0o7777;
            if self.private_files && actual & !0o600 != 0 {
                return Err(permission_error(0o600u32, actual));
            }
            if self.private_files {
                stat.st_mode = (stat.st_mode & !0o7777) | 0o600;
                validate_stat_links(&stat, false, Some(id), 0)?;
                if actual != 0o600 {
                    fs::fchmod(&fd, Mode::from_raw_mode(0o600))?;
                }
            }
            validate_file(&fd, Some(id), 0, self.private_files)?;
            Ok(Some(PrivateFile {
                file: fd.into(),
                id,
                links: 0,
                private: self.private_files,
            }))
        }
        #[cfg(not(target_os = "linux"))]
        Ok(None)
    }
    pub(crate) fn link_temp(&self, temp: &mut PrivateFile, name: &OsStr) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            match fs::linkat(&temp.file, "", &self.fd, name, AtFlags::EMPTY_PATH) {
                Ok(()) => (),
                // AT_EMPTY_PATH requires CAP_DAC_READ_SEARCH. Normal users can
                // link the retained O_TMPFILE through its kernel proc descriptor.
                Err(error)
                    if matches!(error, rustix::io::Errno::NOENT | rustix::io::Errno::PERM) =>
                {
                    self.link_temp_via_proc(temp, name)?;
                }
                Err(error) => return Err(error.into()),
            }
            temp.links = 1;
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (temp, name);
            Err(Error::UnsupportedFilesystem {
                capability: "anonymous temporary linking",
            })
        }
    }
    #[cfg(target_os = "linux")]
    fn link_temp_via_proc(&self, temp: &PrivateFile, name: &OsStr) -> Result<()> {
        use std::os::fd::AsRawFd;

        self.check_private()?;
        temp.verify()?;
        // Only this internally constructed descriptor path is followed. The
        // destination stays relative to the validated private directory handle.
        // AtomicFile::materialize verifies the linked inode and permissions.
        let source = format!("/proc/self/fd/{}", temp.file.as_raw_fd());
        fs::linkat(fs::CWD, source, &self.fd, name, AtFlags::SYMLINK_FOLLOW)?;
        Ok(())
    }
    pub(crate) fn publish_temp(
        &self,
        _temp: &PrivateFile,
        name: &OsStr,
        target: &OsStr,
    ) -> Result<()> {
        fs::renameat(&self.fd, name, &self.fd, target)?;
        Ok(())
    }
    pub(crate) fn cleanup_temp(&self, temp: &PrivateFile, name: Option<&OsStr>) -> Result<()> {
        self.check_private()?;
        temp.verify()?;
        if let Some(name) = name {
            match self.open_file(name) {
                Ok(found) if !found.same_object(temp) => return Err(Error::IdentityChanged),
                Err(Error::Io(e)) if e.kind() == io::ErrorKind::NotFound => {
                    return Err(Error::IdentityChanged);
                }
                Err(e) => return Err(e),
                _ => (),
            }
            // POSIX has no descriptor-bound unlink or atomic compare-and-unlink.
            // Even matching metadata cannot authorize a subsequent unlinkat of a
            // mutable name. Preserve a private orphan instead of deleting a replacement.
            return Err(Error::UnsupportedFilesystem {
                capability: "identity-bound named-temp deletion",
            });
        }
        Ok(()) // The last descriptor closes the anonymous inode automatically.
    }
    pub(crate) fn sync_parent(&self, _durability: crate::Durability) -> Result<()> {
        self.check_private()?;
        fs::fsync(&self.fd)?;
        #[cfg(target_os = "macos")]
        fs::fcntl_fullfsync(&self.fd)?;
        self.check_private()
    }
}
impl PrivateFile {
    pub(crate) fn same_object(&self, other: &Self) -> bool {
        self.id == other.id
    }
    #[cfg(test)]
    pub(crate) fn raw_file(&self) -> &File {
        &self.file
    }
    pub(crate) fn sync_atomic(&self, durability: crate::Durability) -> Result<()> {
        match durability {
            crate::Durability::None => Ok(()),
            crate::Durability::Data => self.sync_data(),
            crate::Durability::Full => {
                self.sync_all()?;
                #[cfg(target_os = "macos")]
                fs::fcntl_fullfsync(&self.file)?;
                self.verify()
            }
        }
    }
}
