# Official NeoForge installer metadata

Retrieved on 2026-10-27 from
https://maven.neoforged.net/releases/net/neoforged/neoforge/21.1.264/neoforge-21.1.264-installer.jar.

Installer size: 6,992,411 bytes. SHA-1, independently compared with the official
`.jar.sha1` sidecar: `f801601bafa7f5957295353930e2cb61dcfddc6c`.
Observed SHA-256: `d7c035dd5436a7c524dfbad6df191b0ef34179fd3edfee9dc99262af3b80239b`.

`21.1.264-version.json` preserves the embedded document bytes.
`21.1.264-install-profile.json` preserves the embedded metadata values except
the decorative base64 icon, which is removed, and JSON whitespace, which is
reformatted. Its hash does not represent the original installer entry.
Neither file contains game binaries, player data, or credentials.

These fixtures exercise the exact Minecraft 1.21.1 / NeoForge 21.1.264 contract.
Tests build synthetic ZIP containers around them to exercise archive validation;
those containers are not the official installer. Neither fixture tests nor a
successful live metadata decode qualify installation, processor execution, or
gameplay on any platform.
