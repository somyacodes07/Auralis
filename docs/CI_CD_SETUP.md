# Auralis GitHub Actions

The repository has two workflows:

- [`ci.yml`](../.github/workflows/ci.yml) runs on pull requests targeting `main` and pushes to `main`.
- [`release.yml`](../.github/workflows/release.yml) runs for tags beginning with `v`. It validates the tag before building and creates a draft release after both native builds finish.

## Pull request and main-branch checks

The CI workflow uses least-privilege `contents: read` permissions and cancels older runs for the same ref. Formatting and patch hygiene run once; Windows and macOS each run Clippy, the workspace tests, and a release build on native hosted runners. Cargo is locked during native checks. The patch-hygiene job checks the actual PR or push range instead of the clean checkout diff.

GitHub Actions are pinned to full commit SHAs. Dependabot checks the GitHub Actions ecosystem weekly and can propose reviewed pin updates.

These checks prove compilation and automated tests on the runners. They do not prove Windows `WorkerW` desktop layering, macOS wallpaper playback, or any other behavior that requires an interactive desktop acceptance test.

The Windows WorkerW lifecycle test runs when the current session exposes a visible shell desktop surface. A headless service session reports that prerequisite and skips only the desktop-layering assertions; an interactive session still exercises the real host creation, resize, and teardown path.

## Draft releases

Only push a matching annotated SemVer tag after its commit is on the default branch. For example, when `Cargo.toml` declares `0.2.0-beta.1`:

```powershell
git push origin main
git tag -a v0.2.0-beta.1 -m "Auralis v0.2.0-beta.1"
git push origin v0.2.0-beta.1
```

The workflow checks tag syntax, exact version alignment with `Cargo.toml`, and reachability from the repository's default branch. It then runs formatting, Clippy, tests, and release builds on Windows and macOS. Windows packaging produces an x64 ZIP and checksum. macOS packaging produces a single-architecture CLI tarball and checksum; it is not a universal binary or a wallpaper app. The final job has `contents: write` permission and creates a **draft**, which must be reviewed and published manually after platform acceptance testing.

The current macOS build supports status and video validation only. Windows desktop behavior still requires manual testing with real video and desktop layering; the exact acceptance matrix should be expanded as supported Windows versions are established.

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
