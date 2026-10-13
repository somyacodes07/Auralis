# Project Aurora — GitHub CI/CD Setup Notes

This file complements `AGENTS.md` and [MULTI_AGENT_WORKFLOW.md](file:///c:/Users/Somyajeet/Code/Git/Auralis/docs/MULTI_AGENT_WORKFLOW.md). It describes the repository automation policy. Workflows are implemented in `.github/workflows/ci.yml` and `.github/workflows/release.yml`.

## Pull request and default-branch CI

Create `.github/workflows/ci.yml` with:

- Triggers: `pull_request` and `push` to the repository's actual default branch.
- A macOS runner and a Windows runner for native compilation and platform-appropriate checks.
- Stable Rust installed according to `rust-toolchain.toml`.
- Cargo caching.
- Formatting, Clippy, unit/integration tests, and release compilation.
- Least-privilege token permissions (`contents: read` unless a job truly needs more).
- `timeout-minutes` on jobs to prevent stuck runners.
- Failure logs/artifacts when useful.
- No secrets for ordinary test builds.

Start with the repository's supported toolchain and commands. If native linking or platform-specific dependencies mean a command differs per OS, use explicit matrix conditions rather than hiding failures.

Example job steps to adapt after inspecting the workspace:

```yaml
- name: Check formatting
  run: cargo fmt --all -- --check

- name: Clippy
  run: cargo clippy --workspace --all-targets -- -D warnings

- name: Test
  run: cargo test --workspace

- name: Release build
  run: cargo build --workspace --release
```

Do not blindly use `--all-features` if features are mutually exclusive or unsupported on a target. Add appropriate target-specific conditions and document why.

## Release workflow

Create a separate `.github/workflows/release.yml` only when packaging is genuinely ready. It should:

1. Trigger on a version tag such as `v0.1.0-beta.1` or `v0.1.0`.
2. Verify the tag version matches the application's declared version.
3. Re-run the required quality gates on supported native runners.
4. Build platform-specific artifacts only on the appropriate runner.
5. Attach checksums and useful build metadata.
6. Publish a GitHub Release only through a deliberate, documented release job.
7. Never auto-publish from ordinary branch pushes.
8. Avoid code-signing secrets until signing is implemented and securely configured.

The exact packaging format is an implementation decision. Do not promise a signed/notarized macOS installer or a Windows installer until the required packaging and credentials have been tested.

## Post-push verification

After each authorized push, the coding agent should check the Actions run for the exact pushed commit, wait for required jobs when practical, and inspect failure logs. A successful `git push` does not mean CI passed.

If the GitHub CLI is installed and authenticated, useful commands include:

```bash
git status --short
git rev-parse HEAD
gh run list --commit "$(git rev-parse HEAD)"
gh run watch
```

Use `gh run watch` only when the relevant run has been identified and the CLI is authenticated. Otherwise use the repository's connected GitHub tooling or report that post-push verification was unavailable.

## Required branch protection (configure in GitHub settings)

- Protect the default branch.
- Require pull requests for changes when practical.
- Require the CI checks that actually exist and are stable.
- Block force pushes and branch deletion on the default branch.
- Require review if the project has maintainers beyond the author.

Branch protection is a repository setting; adding a YAML workflow alone does not enable it.
