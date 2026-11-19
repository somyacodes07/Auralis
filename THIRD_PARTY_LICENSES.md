# Third-party dependency license inventory

This list is generated from `cargo metadata --format-version 1` for the locked dependency graph in `Cargo.lock`. Dependency license expressions remain those published by each upstream project. Review this inventory when dependencies change.

| Crate | Version | Declared license |
| --- | --- | --- |
| itoa | 1.0.18 | MIT OR Apache-2.0 |
| memchr | 2.8.3 | Unlicense OR MIT |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 |
| quote | 1.0.47 | MIT OR Apache-2.0 |
| serde | 1.0.229 | MIT OR Apache-2.0 |
| serde_core | 1.0.229 | MIT OR Apache-2.0 |
| serde_derive | 1.0.229 | MIT OR Apache-2.0 |
| serde_json | 1.0.154 | MIT OR Apache-2.0 |
| syn | 3.0.9 | MIT OR Apache-2.0 |
| unicode-ident | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 |
| windows-sys | 0.59.0 | MIT OR Apache-2.0 |
| windows-targets | 0.52.6 | MIT OR Apache-2.0 |
| windows_aarch64_gnullvm | 0.52.6 | MIT OR Apache-2.0 |
| windows_aarch64_msvc | 0.52.6 | MIT OR Apache-2.0 |
| windows_i686_gnu | 0.52.6 | MIT OR Apache-2.0 |
| windows_i686_gnullvm | 0.52.6 | MIT OR Apache-2.0 |
| windows_i686_msvc | 0.52.6 | MIT OR Apache-2.0 |
| windows_x86_64_gnu | 0.52.6 | MIT OR Apache-2.0 |
| windows_x86_64_gnullvm | 0.52.6 | MIT OR Apache-2.0 |
| windows_x86_64_msvc | 0.52.6 | MIT OR Apache-2.0 |
| zmij | 1.0.23 | MIT |

## Build tooling evaluated and not added

**`embed-resource` 3.0.12 (Windows icon and version resources), evaluated 2026-11-19, not adopted.** It is MIT licensed, actively maintained, supports Rust 1.76+, and finds `rc.exe` for MSVC through the registry and `vswhom`. It was not added because the job needs only one resource compiler call: `build.rs` writes the `.rc` script and runs `rc.exe` (MSVC) or `windres` (GNU) directly, with no build dependency. Adding the crate would bring `cc`, `rustc_version`, `semver`, `toml` and its parser crates, plus `vswhom`, `vswhom-sys` (a compiled C shim), and `winreg` on MSVC, into the locked build graph. Like the direct approach, it also cannot build resources with a 32-bit-only `windres`. Revisit if resource needs grow (for example a manifest, localized strings, or ARM64 cross-builds).

The project itself is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE). Consult each crate's package metadata for the complete license terms.
