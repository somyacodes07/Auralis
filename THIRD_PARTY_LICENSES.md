# Third-party dependency license inventory

This list is generated from `cargo metadata --format-version 1` for the locked dependency graph in `Cargo.lock`. Dependency license expressions remain those published by each upstream project. Review this inventory when dependencies change.

License compliance is also enforced automatically: `cargo deny check` (configured in [`deny.toml`](deny.toml), run in CI) allows only MIT, Apache-2.0, Unicode-3.0, and Unlicense, all compatible with distributing Auralis under MIT OR Apache-2.0. The table below covers every platform in the lock file; `cargo deny` checks only the shipped targets (Windows x64 and macOS), so it reports `windows_x86_64_msvc` but not the other `windows_*` target crates. The Windows installer and portable zip ship this file, `LICENSE-MIT`, and `LICENSE-APACHE` next to `auralis.exe`.

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

The project itself is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE). Consult each crate's package metadata for the complete license terms.
