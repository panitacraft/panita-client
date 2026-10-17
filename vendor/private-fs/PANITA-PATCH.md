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
parent sync and uncertain-commit reporting remain in force. Windows code is
unchanged. No process capabilities or elevated Linux user are required.

Regression tests exercise descriptor publication, destination collisions,
redirected entries, changed permissions and a renamed parent directory. Run
the original dependency tests as well as the launcher tests under a normal user.
The upstream unexpected-alias fault fixture also uses the proc-descriptor method
to inject its alias without privileged capabilities; its rejection assertion is
unchanged.

Reference: https://man7.org/linux/man-pages/man2/link.2.html

Remove this patch only after an upstream release includes an equivalent fix and
passes the same storage, migration, concurrency and Windows regression checks.
