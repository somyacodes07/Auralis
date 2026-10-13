# Auralis Local Verification Suite (PowerShell)
# Implements required quality gates from AGENTS.md Section 5

$ErrorActionPreference = "Stop"

Write-Host "========================================" -ForegroundColor Cyan
Write-Host "  Auralis Local Quality Gate Verification" -ForegroundColor Cyan
Write-Host "========================================" -ForegroundColor Cyan

# 1. Verify Cargo / Rust toolchain availability
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    if (Test-Path "$HOME\.cargo\bin\cargo.exe") {
        $env:Path = "$HOME\.cargo\bin;" + $env:Path
    }
}
if ((Test-Path "C:\MinGW\bin\gcc.exe") -and (-not (Get-Command gcc -ErrorAction SilentlyContinue))) {
    $env:Path = "C:\MinGW\bin;" + $env:Path
}

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host "[ERROR] 'cargo' is not found in PATH." -ForegroundColor Red
    Write-Host "Please install Rust (https://rustup.rs/) and ensure cargo is in PATH." -ForegroundColor Yellow
    exit 1
}

$FailedGates = @()

# 2. Formatting Check
Write-Host "`n[Gate 1/5] Checking Formatting (cargo fmt --all -- --check)..." -ForegroundColor Yellow
try {
    & cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { $FailedGates += "Formatting (cargo fmt)" }
    else { Write-Host "[PASS] Formatting check passed." -ForegroundColor Green }
} catch {
    $FailedGates += "Formatting (cargo fmt)"
}

# 3. Static Analysis / Clippy
Write-Host "`n[Gate 2/5] Running Clippy (cargo clippy --workspace --all-targets -- -D warnings)..." -ForegroundColor Yellow
try {
    & cargo clippy --workspace --all-targets -- -D warnings
    if ($LASTEXITCODE -ne 0) { $FailedGates += "Clippy (cargo clippy)" }
    else { Write-Host "[PASS] Clippy analysis passed." -ForegroundColor Green }
} catch {
    $FailedGates += "Clippy (cargo clippy)"
}

# 4. Unit & Integration Tests
Write-Host "`n[Gate 3/5] Running Tests (cargo test --workspace)..." -ForegroundColor Yellow
try {
    & cargo test --workspace
    if ($LASTEXITCODE -ne 0) { $FailedGates += "Tests (cargo test)" }
    else { Write-Host "[PASS] Test suite passed." -ForegroundColor Green }
} catch {
    $FailedGates += "Tests (cargo test)"
}

# 5. Release Compilation
Write-Host "`n[Gate 4/5] Building Release Target (cargo build --workspace --release)..." -ForegroundColor Yellow
try {
    & cargo build --workspace --release
    if ($LASTEXITCODE -ne 0) { $FailedGates += "Release Build (cargo build --release)" }
    else { Write-Host "[PASS] Release compilation passed." -ForegroundColor Green }
} catch {
    $FailedGates += "Release Build (cargo build --release)"
}

# 6. Patch Hygiene
Write-Host "`n[Gate 5/5] Checking Git Diff Hygiene (git diff --check)..." -ForegroundColor Yellow
try {
    & git diff --check
    if ($LASTEXITCODE -ne 0) { $FailedGates += "Patch Hygiene (git diff --check)" }
    else { Write-Host "[PASS] Git diff hygiene passed." -ForegroundColor Green }
} catch {
    $FailedGates += "Patch Hygiene (git diff --check)"
}

Write-Host "`n========================================" -ForegroundColor Cyan
if ($FailedGates.Count -eq 0) {
    Write-Host "[ALL GATES PASSED] Working tree is verified and ready for push." -ForegroundColor Green
    exit 0
} else {
    Write-Host "[FAILED GATES]: $($FailedGates -join ', ')" -ForegroundColor Red
    Write-Host "Do not push until all required quality gates pass." -ForegroundColor Red
    exit 1
}
