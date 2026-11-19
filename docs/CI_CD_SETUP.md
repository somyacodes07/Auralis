# Auralis GitHub Actions

The repository has four workflows:

- [`ci.yml`](../.github/workflows/ci.yml) runs on pull requests targeting `main` and pushes to `main`.
- [`build-release.yml`](../.github/workflows/build-release.yml) runs on every push to `main` (usually a merged pull request) and publishes a numbered prerelease.
- [`release.yml`](../.github/workflows/release.yml) runs for maintainer tags such as `v1.0.0-rc.1`. It validates the tag before building and creates a draft release after both native builds finish.
- [`package.yml`](../.github/workflows/package.yml) is the reusable build both release workflows call. It runs formatting, Clippy, tests, and a release build on Windows and macOS, then packages the binaries with SHA-256 checksums.

## Numbered builds from main

Each push to `main` builds the app and publishes a GitHub **prerelease** named `Auralis <version> · Build <N>` with the tag `v<version>-build.<N>`, for example `v1.0.0-rc.1-build.42` (no `+`, which GitHub would show as `%2B` in the release URL). `<N>` is the workflow run number, so it increases with every build. `<version>` is read from `Cargo.toml`, so the package version is not bumped for each build. The build number is stamped into the binary through `AURALIS_BUILD_NUMBER`: the Studio title, tray tooltip, and `auralis status` show `1.0.0-rc.1+build.42`, and the macOS bundle uses it as `CFBundleVersion`. Asset names match the tag, for example `auralis-v1.0.0-rc.1-build.42-windows-x86_64.zip`.

The workflow refuses to publish from any branch other than the default branch, never marks a build as the latest release, and only the final publish job has `contents: write`. Builds can also be started manually from the Actions tab (`workflow_dispatch`) on `main`. These are unsigned test builds; `release.yml` ignores `-build.<N>` tags so they never become draft releases.

## Pull request and main-branch checks

The CI workflow uses least-privilege `contents: read` permissions and cancels older runs for the same ref. Formatting and patch hygiene run once; Windows and macOS each run Clippy, the workspace tests, and a release build on native hosted runners. Cargo is locked during native checks, and all declared crate features are checked. The patch-hygiene job checks the actual PR or push range instead of the clean checkout diff.

Pull requests also run `package.yml` (the **Package Installers** check), so the Windows installer, Windows zip, and universal macOS DMG are built and attached to the workflow run before merge. Nothing is published from a pull request. Docs-only pull requests (every changed file is Markdown, under `docs/`, or under `site/`) skip packaging: the **Detect Docs-Only Changes** job compares the PR with its base, and **Package Installers** shows as skipped. Formatting, Clippy, tests, and release builds still run for every pull request.

The **Supply Chain (cargo-deny)** job runs [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny) against [`deny.toml`](../deny.toml) for the Windows x64 and macOS targets. It fails on RustSec advisories (vulnerable, unsound, or unmaintained crates), yanked versions, licenses outside the allow-list (MIT, Apache-2.0, Unicode-3.0, Unlicense), and crates from anywhere other than crates.io. Duplicate crate versions are reported as warnings only. To accept an advisory, add it to `[advisories] ignore` in `deny.toml` with a reason and record it in the pull request. Run it locally with `cargo deny check` after `cargo install cargo-deny --locked` (or a prebuilt binary from the cargo-deny releases page).

## Windows installer

[`installer/auralis.iss`](../installer/auralis.iss) is an [Inno Setup 6](https://jrsoftware.org/isinfo.php) script. The Windows job of `package.yml` uses the runner's preinstalled `ISCC.exe` or installs Inno Setup with Chocolatey, then compiles it with `/DAppVersion=<Cargo.toml version>`, `/DBuildNumber=<N>` for numbered builds, and `/DOutputBaseName=auralis-<label>-windows-x86_64-setup`. The file version resource is `MAJOR.MINOR.PATCH.<build>` (`.0` for tag releases).

The installer is per-user (`PrivilegesRequired=lowest`, no administrator prompt) and installs to `%LOCALAPPDATA%\Programs\Auralis`. Its `AppId` GUID is fixed and must never change, or upgrades will install side by side. Before installing, upgrading, or uninstalling it runs `auralis.exe quit` from the install folder, then force-terminates any `auralis.exe` still running from that folder. The optional "Start Auralis when I sign in" task writes the same `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` value (`Auralis` = `"<exe>" gui --minimized`) as the in-app toggle; the uninstaller always removes it. Settings (`%APPDATA%\Auralis`) and logs (`%LOCALAPPDATA%\Auralis\logs`) are deleted only if the user answers **Yes** in an interactive uninstall. There is no license-acceptance page because MIT and Apache-2.0 do not require acceptance; the license files are installed next to `auralis.exe`.

After compiling, CI smoke-tests the installer on the runner: a silent install with the sign-in task, checks for the installed files, Start menu shortcut, and Run value, runs `auralis.exe status`, then a silent uninstall that must remove the files, shortcut, and Run value while keeping the settings file. Logs are uploaded as an `installer-logs` artifact if this fails. The smoke test does not cover the interactive wizard, upgrading over a running Studio, or SmartScreen; those are manual checks.

To build the installer locally, run `cargo build --release`, then `"C:\Program Files (x86)\Inno Setup 6\ISCC.exe" /DAppVersion=1.0.0-rc.1 installer\auralis.iss`; the output goes to `dist\`.

GitHub Actions are pinned to full commit SHAs. Dependabot checks the GitHub Actions ecosystem weekly and can propose reviewed pin updates.

These checks prove compilation and automated tests on the runners. They do not prove Windows `WorkerW` or macOS desktop layering, visible video playback, or any other behavior that requires an interactive desktop acceptance test. The macOS job compiles the Objective-C AppKit/AVFoundation bridge through `build.rs` and links the native frameworks.

The Windows WorkerW lifecycle test runs in the active interactive console session with a visible shell desktop surface. A service, headless, or non-console session reports that prerequisite and skips only the desktop-layering assertions; an active console session still exercises the real host creation, resize, and teardown path.

## Draft releases

Only push a matching annotated SemVer tag after its commit is on the default branch. For example, when `Cargo.toml` declares `1.0.0-rc.1`:

```powershell
git push origin main
git tag -a v1.0.0-rc.1 -m "Auralis v1.0.0-rc.1"
git push origin v1.0.0-rc.1
```

The workflow checks tag syntax, exact version alignment with `Cargo.toml`, and reachability from the repository's default branch. It then runs formatting, Clippy, tests, and release builds on Windows and macOS. Windows packaging produces an x64 installer and a portable ZIP, each with a checksum (see [Windows installer](#windows-installer)). macOS packaging builds Apple Silicon and Intel binaries, merges them with `lipo` into one universal `Auralis.app` (minimum macOS 13), ad-hoc signs it, and wraps it in a drag-to-Applications `.dmg` with a checksum. The app is not notarized. The final job has `contents: write` permission and creates a **draft**, which must be reviewed and published manually after platform acceptance testing.

The macOS menu-bar app uses AVFoundation to play local video in a native desktop host, but visual desktop layering, playback, display changes, and sleep/wake still require manual testing on a real Mac. Windows desktop behavior also requires testing with real media and the interactive desktop. See [the platform test matrix](PLATFORM_TEST_MATRIX.md).

## GitHub repository rules

Workflow YAML cannot enable branch protection. In GitHub repository settings, protect `main`, require pull requests, block force pushes and branch deletion, and require the CI checks (`Code Formatting`, `Patch & Workspace Hygiene`, `Supply Chain (cargo-deny)`, `Windows Native Verification`, and `macOS Native Verification`). Restrict creation of `v*` release tags to maintainers who can review releases. These repository settings were not changed by this code update.

## Local verification

Run the canonical five-gate script before submitting or pushing code:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify.ps1
```

```bash
bash scripts/verify.sh
```

Do not create a release tag or publish a draft until the version and changelog have been reviewed under the policy in [`AGENTS.md`](../AGENTS.md).
