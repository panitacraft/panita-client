# Local private-fs patch

Base: crates.io private-fs 0.1.0, upstream commit
`3c9a9811c6f1b133ed34e094d1a401fb22f888f0`.
Published archive SHA-256:
`49f6bde7763e3eaa66974b8e9c1580d06c34c2197353ca7362185d69419471a4`.
Original source, manifests, tests, benchmarks and both licenses are retained.
This directory is a pinned Cargo patch, not an upstream release.

Linux anonymous temporary publication first attempts the original
`linkat(AT_EMPTY_PATH)`. On ENOENT or EPERM it uses the documented normal-user
alternative: link the held file descriptor through `/proc/self/fd/<fd>` with
AT_SYMLINK_FOLLOW. Only the internally constructed kernel descriptor path is
followed; destination names stay relative to the validated private directory.
Missing/inaccessible procfs fails closed. Other link errors are not retried.

The temporary remains anonymous until linked, private from creation and on the
same filesystem. Existing temporary/directory validation, linked-inode identity
and permission checks, file sync, atomic rename, post-publication validation,
parent sync and uncertain-commit reporting remain in force. No process
capabilities or elevated Linux user are required.

Regression tests exercise descriptor publication, destination collisions,
redirected entries, changed permissions and a renamed parent directory. Run
the original dependency tests as well as the launcher tests under a normal user.
The upstream unexpected-alias fault fixture also uses the proc-descriptor method
to inject its alias without privileged capabilities; its rejection assertion is
unchanged.

Reference: https://man7.org/linux/man-pages/man2/link.2.html

Windows drive-root acquisition resolves the current DOS drive mapping with
`QueryDosDeviceW` before opening the direct `\Device\<device>\` anchor. The
reported Windows 10 startup failure was `STATUS_REPARSE_POINT_ENCOUNTERED`
(`0xC000050B`, Win32 4395) at the original `\??\C:\` drive-root open, before
profile creation. The normal DOS drive alias must be resolved separately from
filesystem traversal. Only a single direct device name is accepted; SUBST,
network paths, chained aliases and directory suffixes fail closed. Historical
device mappings are never tried. The returned handle pins the selected volume;
subsequent opens do not resolve the drive letter again.

`OBJ_DONT_REPARSE`, `FILE_OPEN_REPARSE_POINT`, handle kind checks, volume ACL
checks and component-by-component ancestor traversal remain enabled. There is no
retry with relaxed flags and no permission repair or elevation. Tests cover fresh
private-store creation/reopening, invalid mapping shapes, and rejection of a
junction ancestor without creating files at its destination. Run the dependency
and launcher storage tests on Windows; an installed candidate still needs a
normal-user clean Windows 10 VM check before calling this report resolved.

Local verification on 2026-10-28: all 51 dependency unit tests pass on Windows 11
build 26200 outside the execution sandbox; all six launcher storage tests and
library Clippy with warnings denied pass as well. The sandbox denies the raw
drive-root access needed by these tests.
These results do not qualify Windows 10 or a packaged launcher build.

The owner's follow-up probe on Windows 10 22H2 build 19045.3803 passes root
creation/reopening and directory flushing, but both classic rename and initial
atomic publication fail with Win32 32 (`ERROR_SHARING_VIOLATION`). Newly ensured
directories previously retained `FILE_ALL_ACCESS`, including data-write and
DELETE access, which can conflict with the rename operation's internal target
directory open. All retained directory handles now request metadata, traversal,
security-read and synchronization access only, including when creating a private
directory. The protected owner-only DACL is unchanged. File creation retains its
write/delete access; explicit directory flush and ACL operations acquire brief
handle-relative writable handles. No sharing error is ignored or retried with
weaker publication or durability guarantees.

The regression holds newly created and re-ensured parent/child capabilities
while opening a writer that shares read access only, then performs a durable
write. All 52 dependency tests and 404 core tests pass on Windows 11 (one core
test remains ignored); library Clippy also passes. Repeat the updated probe and
installed-launcher account-save/restart checks on Windows 10 before qualification.

References:
- https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-querydosdevicew
- https://learn.microsoft.com/en-us/windows/win32/api/ntdef/ns-ntdef-_object_attributes
- https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/ns-ntifs-_file_rename_information

Remove this patch only after an upstream release includes an equivalent fix and
passes the same storage, migration, concurrency and Windows regression checks.
