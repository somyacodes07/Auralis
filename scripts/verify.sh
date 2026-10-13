#!/usr/bin/env bash
# Auralis Local Verification Suite (POSIX Shell)
# Implements required quality gates from AGENTS.md Section 5

set -euo pipefail

echo "========================================"
echo "  Auralis Local Quality Gate Verification"
echo "========================================"

if ! command -v cargo &> /dev/null; then
    echo "[ERROR] 'cargo' is not found in PATH."
    echo "Please install Rust (https://rustup.rs/) and ensure cargo is in PATH."
    exit 1
fi

FAILED_GATES=()

echo ""
echo "[Gate 1/5] Checking Formatting (cargo fmt --all -- --check)..."
if ! cargo fmt --all -- --check; then
    FAILED_GATES+=("Formatting")
else
    echo "[PASS] Formatting check passed."
fi

echo ""
echo "[Gate 2/5] Running Clippy (cargo clippy --workspace --all-targets -- -D warnings)..."
if ! cargo clippy --workspace --all-targets -- -D warnings; then
    FAILED_GATES+=("Clippy")
else
    echo "[PASS] Clippy analysis passed."
fi

echo ""
echo "[Gate 3/5] Running Tests (cargo test --workspace)..."
if ! cargo test --workspace; then
    FAILED_GATES+=("Tests")
else
    echo "[PASS] Test suite passed."
fi

echo ""
echo "[Gate 4/5] Building Release Target (cargo build --workspace --release)..."
if ! cargo build --workspace --release; then
    FAILED_GATES+=("Release Build")
else
    echo "[PASS] Release compilation passed."
fi

echo ""
echo "[Gate 5/5] Checking Git Diff Hygiene (git diff --check)..."
if ! git diff --check; then
    FAILED_GATES+=("Patch Hygiene")
else
    echo "[PASS] Git diff hygiene passed."
fi

echo ""
echo "========================================"
if [ ${#FAILED_GATES[@]} -eq 0 ]; then
    echo "[ALL GATES PASSED] Working tree is verified and ready for push."
    exit 0
else
    echo "[FAILED GATES]: ${FAILED_GATES[*]}"
    echo "Do not push until all required quality gates pass."
    exit 1
fi
