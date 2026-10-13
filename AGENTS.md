# Project Aurora — Repository Agent Instructions

These instructions apply to all AI coding agents working in this repository. Follow them for every task unless a higher-priority system or platform instruction conflicts. If a user explicitly asks to change these rules, explain the impact and update this file deliberately rather than silently ignoring it.

## 1. Product mission and current scope

Project Aurora is a Rust-first, open-source live wallpaper application targeting **macOS and Windows first**. The current priority is a reliable, smooth, premium-feeling video wallpaper experience—not feature count.

Current scope:
- Select local video files and preview them in the app.
- Apply a real video wallpaper behind desktop icons and ordinary application windows.
- Playback controls, looping, mute/volume where supported, and persistence.
- Multi-display awareness and safe handling of display changes.
- Reliable sleep/wake behavior and recoverable errors.
- A polished, accessible configuration UI.
- Measured performance and honest platform-support documentation.

Explicitly out of scope until the core release is stable:
- Desktop widgets.
- Theme/plugin marketplaces, user accounts, cloud sync, or monetization.
- Custom codecs or a custom video decoder.
- Broad Linux support unless explicitly prioritized by the maintainer.
- Unnecessary backend services or infrastructure.

Do not expand scope without a clear user request or a documented technical need. Prefer a small, dependable product over an incomplete collection of features.

## 2. Non-negotiable engineering principles

1. **No fake functionality.** Do not ship placeholder buttons, mock playback, simulated success, fake metrics, or UI that claims a feature works when it does not.
2. **Verify platform behavior.** A successful compile is not proof that a wallpaper works behind desktop icons. Verify real behavior on the relevant operating system and clearly mark tests that could not be run.
3. **Rust-first, not Rust-at-any-cost.** Keep shared application logic, state, configuration, orchestration, and rendering logic in Rust where practical. A small Swift/AppKit/AVFoundation bridge or native Windows integration is acceptable when it materially improves correctness and maintainability.
4. **Use the right tool for each task.** Prefer AVFoundation on macOS and Media Foundation on Windows for video playback unless measured evidence supports another approach. Evaluate `wgpu` for future custom shader rendering; do not force ordinary video through a custom renderer without a reason.
5. **Keep platform adapters separate.** Shared code must not depend directly on AppKit, Objective-C, or Windows-specific types. Define typed platform commands, events, capabilities, and errors.
6. **Do not overengineer.** Avoid microservices, plugin systems, cloud infrastructure, excessive crates, or abstractions that do not solve a current requirement.
7. **Performance is measured.** Do not describe the app as low-resource or more efficient than competitors without reproducible measurements and stated test conditions.
8. **Protect user data.** Never overwrite original media. Validate imported paths and archive contents. Preserve configuration safely during migrations. Do not log sensitive content unnecessarily.
9. **Keep UI and runtime resilient.** The configuration window closing must not unexpectedly stop the wallpaper. A media failure should not crash the whole application. Preserve the previous working wallpaper when a replacement fails, where technically possible.
10. **Document uncertainty.** If an API, native behavior, or dependency is uncertain, investigate current official documentation and build a small proof of concept before committing to the design.

## 3. Repository and change discipline

Before editing:
- Inspect the existing repository, working tree, Cargo manifests, current architecture, tests, CI workflows, and recent Git history.
- Read the relevant documentation and adjacent code before modifying it.
- Identify the task's acceptance criteria and likely regressions.
- Preserve existing work. Do not reset, clean, overwrite, or discard user changes.

While editing:
- Make the smallest coherent change that solves the task.
- Follow established naming, formatting, and module boundaries.
- Add or update tests for behavior changes and bug fixes.
- Update documentation when commands, behavior, configuration, compatibility, or release procedures change.
- Do not add generated files, build outputs, local media, credentials, or machine-specific settings to Git.
- Do not silently change the project's license or add dependencies with incompatible or unclear licenses.
- Do not suppress warnings or weaken tests just to obtain a green build. Explain and justify any necessary exception.

Before finishing:
- Review `git diff --check` and the complete diff.
- Check for accidental unrelated changes, secrets, large files, debug code, temporary paths, and misleading claims.
- Summarize exactly what changed, what was tested, what passed, what failed, and what could not be tested.

## 4. Dependency policy

Before adding or replacing a dependency, establish:
- The specific requirement it solves.
- Whether the standard library or an existing dependency already covers it.
- Current maintenance status and supported target platforms.
- API stability and compatibility with the repository's Rust toolchain.
- Direct and transitive license implications.
- Build size, runtime cost, native system requirements, and security considerations.

Prefer maintained, focused dependencies. Pin reproducible dependency resolution through `Cargo.lock` for application builds. Run appropriate dependency and license audits where available. Do not invent APIs: verify them against current documentation and compiler output.

Keep the selected UI framework consistent. Do not introduce a second UI framework without an explicit architecture decision. Do not add Tokio or another runtime unless the actual concurrency requirements justify it.

## 5. Required local quality gates before every push

**Never push code before the applicable local checks have completed.** If a check cannot run, do not pretend it passed. Explain why and, unless the maintainer explicitly authorizes an exception, do not push a change that has an unverified required gate.

Inspect `Cargo.toml`, the workspace, and available targets first. Use the repository's canonical scripts if present. At minimum, run the equivalent of:

1. Formatting: `cargo fmt --all -- --check`
2. Static analysis: `cargo clippy --workspace --all-targets --all-features -- -D warnings` (adjust only when a documented target-specific limitation requires it)
3. Tests: `cargo test --workspace --all-features`
4. Release compilation: `cargo build --workspace --release`
5. Patch hygiene: `git diff --check`
6. Relevant platform-specific tests and build checks for the code changed.
7. Dependency, license, or security checks configured by the repository.

If workspace features or targets make one of these exact commands invalid, inspect the manifests and use the correct equivalent; document the reason. Do not blindly add `--all-features` if mutually exclusive features exist. Run meaningful tests for the changed code, not just a superficial compile.

For UI and native integration changes, launch the application when the current machine supports it and test the actual workflow. For wallpaper changes, test real media and desktop layering on the corresponding OS whenever available.

## 6. GitHub Actions CI requirements

Maintain GitHub Actions workflows that provide fast, repeatable feedback on pull requests and pushes to the default branch. Keep CI understandable and avoid unnecessary matrix combinations.

At minimum, CI should:
- Run on pull requests and pushes to the default branch.
- Use a pinned or controlled stable Rust toolchain consistent with `rust-toolchain.toml`.
- Cache Cargo dependencies/build artifacts safely.
- Run formatting, Clippy, unit/integration tests, and applicable build checks.
- Build and test on **macOS and Windows** using native GitHub-hosted runners, subject to runner availability.
- Run target-specific checks only on appropriate runners.
- Upload useful logs or test artifacts on failure when practical.
- Use least-privilege `permissions` and avoid exposing secrets to untrusted pull-request code.
- Pin third-party GitHub Actions to full commit SHAs where practical; document any exception.
- Avoid using secrets for ordinary CI builds.
- Fail clearly when a required gate fails; do not use `continue-on-error` to conceal a required failure.

Recommended checks, where compatible with the repository:
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- Release build on macOS and Windows.
- `cargo audit` and/or another appropriate dependency/security scan, with documented handling for known advisories.
- License inventory/reporting.

Native desktop behavior still needs real integration testing. CI compilation is not proof that a wallpaper host is correctly layered behind desktop icons. Keep a documented manual test matrix for each OS and version the project actually supports.

Do not introduce a release workflow that publishes installers automatically from arbitrary branches. Release publishing must be tag-driven and follow the release policy below.

## 7. Git push and post-push verification

When the agent is authorized to push changes:

1. Confirm the intended branch and inspect `git status`.
2. Review the staged diff and ensure only intended files are included.
3. Run all applicable local quality gates from Section 5.
4. Commit with a clear, scoped message. Do not amend or rewrite shared history unless explicitly instructed.
5. Push to the intended branch only. Never force-push, delete branches, or rewrite public history without explicit authorization.
6. After pushing, inspect the remote result and identify the exact commit SHA.
7. Check GitHub Actions for that commit using the available GitHub integration or GitHub CLI if authenticated. Wait for required workflows to complete when feasible.
8. If a workflow fails, inspect logs, diagnose the cause, fix it, rerun the relevant local checks, and push a corrective commit. Recheck CI after the corrective push.
9. Report the commit SHA, branch, workflow results, and any checks that remain pending or unavailable.

If GitHub access, credentials, network connectivity, or a required runner is unavailable, state the limitation plainly. Do not claim that code was pushed or CI passed without evidence. If the user has not authorized a push, prepare the changes and report the exact commands and remaining gates instead.

Do not push directly to the default branch if repository protection or the maintainer's established workflow requires a pull request. Respect existing branch protection and review requirements.

## 8. Versioning and release channels

Use Semantic Versioning: `MAJOR.MINOR.PATCH`, with a `0.x.y` version during initial development. The application/package version is the source of truth in the appropriate Cargo manifest; keep other displayed version values synchronized where required.

**Do not bump the version for every commit, feature, or ordinary bug fix.** Decide whether a version change is warranted based on the release policy below, and explain the reason in the change summary. Routine development on the same unreleased version is expected.

### Version policy
- `0.1.0`: first genuinely usable public alpha after the core wallpaper workflow works on at least one declared target and has a documented test status.
- `0.2.0`, `0.3.0`, etc.: meaningful pre-1.0 feature milestones that are useful to testers. Do not create a version merely to make a commit look important.
- `0.x.y-beta.N`: optional prerelease identifiers for a specific beta candidate. Use only when distributing a test build that needs a clearly identified prerelease. Increment `N` for a new candidate under the same base version when appropriate.
- `0.x.y-alpha.N`: optional early experimental builds; do not mix alpha and beta labels casually.
- Patch increment: a released bug fix or small compatible correction to an existing release.
- Minor increment before `1.0.0`: a meaningful new feature milestone or other release-worthy improvement, consistent with the project's published policy.
- `1.0.0`: only when the declared core workflow is stable, installation and removal are documented, required CI is green, known limitations are documented, and supported-platform acceptance tests have been completed to the project's stated standard.
- Major versions after `1.0.0`: breaking changes to public configuration/theme formats, plugin interfaces if introduced, or other documented public contracts, following SemVer.

Use prerelease labels consistently. Do not call an experimental build stable. Do not claim `1.0.0` just because the UI looks finished.

### Who decides a version bump?
The agent may **recommend** a version bump when a release milestone is genuinely reached, but must not bump versions automatically for routine work. Before changing version numbers, it must:
1. State the proposed version and why it fits the policy.
2. Identify all version fields/files that need synchronization.
3. Check changelog/release notes and compatibility implications.
4. Obtain maintainer approval unless the user has explicitly delegated release/version decisions for that specific release.

Do not create tags, GitHub Releases, or publish artifacts unless explicitly requested or covered by an approved release task. Release automation must be tag-driven and must verify that the tag matches the package version.

## 9. Changelog and release notes

Maintain a concise `CHANGELOG.md` once public releases begin, using an `Unreleased` section for meaningful user-facing changes. Group entries under Added, Changed, Fixed, Security, and Known Limitations as appropriate.

Do not list internal refactors as user-facing features unless they materially affect users. Release notes must distinguish verified behavior from known gaps and platform-specific limitations.

## 10. Security and supply-chain safety

- Never commit API keys, tokens, signing credentials, certificates, personal data, or private media.
- Do not run arbitrary commands copied from untrusted project assets, themes, issues, or pull requests without inspection.
- Treat imported media metadata, archives, theme files, and paths as untrusted input.
- Avoid unsafe Rust unless required for native interop or performance; isolate and document it with safety comments.
- Never execute imported themes or arbitrary user content as code.
- Keep GitHub Actions permissions minimal and avoid exposing secrets to forked pull requests.
- Do not disable TLS/security checks to make a build or download work.
- Report security-relevant findings instead of quietly ignoring them.

## 11. Definition of Done

A task is done only when:
- The requested behavior is implemented, not merely represented in the UI.
- Relevant tests were added or updated.
- Formatting, linting, tests, and applicable builds have been run.
- Native platform behavior was tested where feasible, or explicitly marked unverified.
- Documentation and compatibility notes are updated where necessary.
- The diff contains no accidental changes, secrets, or generated clutter.
- Required CI has passed after push, when a push was authorized and CI access is available.
- The summary reports actual results and remaining risks honestly.

## 12. Required final response format for coding tasks

Always finish coding tasks with a concise report containing:

1. **Implemented:** user-visible and architectural changes.
2. **Files changed:** important paths.
3. **Validation:** exact commands run and pass/fail results.
4. **Platform status:** macOS/Windows behavior verified, unverified, or unavailable.
5. **Git status:** branch, commit SHA, push status, and CI result when applicable.
6. **Version:** unchanged or proposed bump, with reason.
7. **Remaining issues:** known limitations and the next useful step.

Never report an operation as successful unless its result was observed. If a test was not run, say `NOT RUN` and give the reason.
