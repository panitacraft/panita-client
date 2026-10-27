# Public Mojang metadata fixtures

Downloaded from Mojang on 2026-10-16. These contain public game metadata only.
The version documents are retained byte for byte and checked against the SHA-1
values recorded during Phase 0 in the unit tests.

| Version | Official document | SHA-1 |
|---|---|---|
| 26.3 | https://piston-meta.mojang.com/v1/packages/702fe59163c6ee6578607daa85811d9bc9c7cc40/26.3.json | 702fe59163c6ee6578607daa85811d9bc9c7cc40 |
| 1.21.1 | https://piston-meta.mojang.com/v1/packages/cedfc3b6dcbca34e2b478d498bf1d56a8fa2f404/1.21.1.json | cedfc3b6dcbca34e2b478d498bf1d56a8fa2f404 |

Fixtures establish resolver behavior, not actual game-launch qualification.

Java manifests come from the public Mojang runtime index. Tests verify these
exact bytes and resolve every manifest file/link on both target platforms:

| Platform/component | Manifest SHA-1 |
|---|---|
| windows-x64 / java-runtime-epsilon | b374544c680d965fb5535977d7cb04c6befe1930 |
| windows-x64 / java-runtime-delta | cb4394a27089d19f65d5baa6cf0482c27c3c7865 |
| linux / java-runtime-epsilon | ccf325ef8f5be596cf8229fd349298d8a74d2388 |
| linux / java-runtime-delta | 06a3884df3d9d4072b9b6df541495d81878ad2de |

Each source URL is
`https://piston-meta.mojang.com/v1/packages/<sha1>/manifest.json`.
