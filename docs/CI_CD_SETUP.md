# Auralis GitHub Actions

The repository has four workflows:

- [`ci.yml`](../.github/workflows/ci.yml) runs on pull requests targeting `main` and pushes to `main`.
- [`build-release.yml`](../.github/workflows/build-release.yml) runs on every push to `main` (usually a merged pull request) and publishes a numbered prerelease.
- [`release.yml`](../.github/workflows/release.yml) runs for maintainer tags such as `v0.2.0-beta.1`. It validates the tag before building and creates a draft release after both native builds finish.
- [`package.yml`](../.github/workflows/package.yml) is the reusable build both release workflows call. It runs formatting, Clippy, tests, and a release build on Windows and macOS, then packages the binaries with SHA-256 checksums.

## Numbered builds from main

Each push to `main` builds the app and publishes a GitHub **prerelease** named `Auralis <version> (build <N>)` with the tag `v<version>+build.<N>`, for example `v0.2.0-beta.1+build.42`. `<N>` is the workflow run number, so it increases with every build. `<version>` is read from `Cargo.toml`; `+build.<N>` is SemVer build metadata, so the package version is not bumped for each build. The build number is stamped into the binary through `AURALIS_BUILD_NUMBER`: the Studio title, tray tooltip, and `auralis status` show `0.2.0-beta.1+build.42`, and the macOS bundle uses it as `CFBundleVersion`. Asset names use `-build.<N>` (no `+`), for example `auralis-v0.2.0-beta.1-build.42-windows-x86_64.zip`.

The workflow refuses to publish from any branch other than the default branch, never marks a build as the latest release, and only the final publish job has `contents: write`. Builds can also be started manually from the Actions tab (`workflow_dispatch`) on `main`. These are unsigned test builds; `release.yml` ignores `+build` tags so they never become draft releases.

## Pull request and main-branch checks

The CI workflow uses least-privilege `contents: read` permissions and cancels older runs for the same ref. Formatting and patch hygiene run once; Windows and macOS each run Clippy, the workspace tests, and a release build on native hosted runners. Cargo is locked during native checks, and all declared crate features are checked. The patch-hygiene job checks the actual PR or push range instead of the clean checkout diff.

GitHub Actions are pinned to full commit SHAs. Dependabot checks the GitHub Actions ecosystem weekly and can propose reviewed pin updates.

These checks prove compilation and automated tests on the runners. They do not prove Windows `WorkerW` or macOS desktop layering, visible video playback, or any other behavior that requires an interactive desktop acceptance test. The macOS job compiles the Objective-C AppKit/AVFoundation bridge through `build.rs` and links the native frameworks.

The Windows WorkerW lifecycle test runs in the active interactive console session with a visible shell desktop surface. A service, headless, or non-console session reports that prerequisite and skips only the desktop-layering assertions; an active console session still exercises the real host creation, resize, and teardown path.

## Draft releases

Only push a matching annotated SemVer tag after its commit is on the default branch. For example, when `Cargo.toml` declares `0.2.0-beta.1`:

```powershell
git push origin main
git tag -a v0.2.0-beta.1 -m "Auralis v0.2.0-beta.1"
git push origin v0.2.0-beta.1
```

The workflow checks tag syntax, exact version alignment with `Cargo.toml`, and reachability from the repository's default branch. It then runs formatting, Clippy, tests, and release builds on Windows and macOS. Windows packaging produces an x64 ZIP and checksum. macOS packaging produces a single-architecture `.app` bundle and CLI tarball with checksums. The app bundle is unsigned and not notarized. The final job has `contents: write` permission and creates a **draft**, which must be reviewed and published manually after platform acceptance testing.

The macOS menu-bar app uses AVFoundation to play local video in a native desktop host, but visual desktop layering, playback, display changes, and sleep/wake still require manual testing on a real Mac. Windows desktop behavior also requires testing with real media and the interactive desktop. See [the platform test matrix](PLATFORM_TEST_MATRIX.md).

## GitHub repository rules

Workflow YAML cannot enable branch protection. In GitHub repository settings, protect `main`, require pull requests, block force pushes and branch deletion, and require the four CI checks (`Code Formatting`, `Patch & Workspace Hygiene`, `Windows Native Verification`, and `macOS Native Verification`). Restrict creation of `v*` release tags to maintainers who can review releases. These repository settings were not changed by this code update.

## Local verification

Run the canonical five-gate script before submitting or pushing code:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/verify.ps1
```

```bash
bash scripts/verify.sh
```

Do not create a release tag or publish a draft until the version and changelog have been reviewed under the policy in [`AGENTS.md`](../AGENTS.md).
