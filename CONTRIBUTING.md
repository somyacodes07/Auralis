# Contributing to Auralis

Thanks for helping improve Auralis. Contributions should keep the core wallpaper workflow reliable, efficient, accessible, and honest about platform support.

## Before opening a pull request

1. Check the issue tracker and current documentation before starting a larger change.
2. Keep changes focused and avoid adding unsupported UI or claims for unfinished behavior.
3. Add or update tests for behavior changes and update user documentation where behavior or platform support changes.
4. Run the applicable local checks:

   ```powershell
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets --locked -- -D warnings
   cargo test --workspace --locked
   cargo build --workspace --release --locked
   git diff --check
   ```

5. For native platform changes, report which OS and interactive desktop behaviors were tested. A CI build does not verify wallpaper layering.
6. Open a pull request with a clear summary, validation results, and known limitations. Do not force-push or rewrite shared history.

## Project conventions

- Shared orchestration and configuration belong in Rust; platform APIs stay behind platform adapters.
- Prefer the system video framework: Media Foundation on Windows and AVFoundation on macOS.
- Do not claim memory or playback improvements without measurements and test conditions.
- Never commit private media, credentials, build outputs, or machine-specific configuration.
- Auralis is dual-licensed under MIT or Apache-2.0. New contributions are accepted under the same terms.

See [AGENTS.md](AGENTS.md) for the full engineering and release policy.
