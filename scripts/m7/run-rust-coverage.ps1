$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$workspaceRoot = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
$coverageToolchain = "nightly-2026-09-01"
$originalTarget = $env:CARGO_LLVM_COV_TARGET_DIR
$originalTestThreads = $env:RUST_TEST_THREADS

Push-Location $workspaceRoot
try {
    & rustup run $coverageToolchain rustc --version | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "Rust coverage requires $coverageToolchain. Install it with: rustup toolchain install $coverageToolchain --profile minimal"
    }

    New-Item -ItemType Directory -Force -Path "coverage" | Out-Null
    $env:RUST_TEST_THREADS = "1"

    $env:CARGO_LLVM_COV_TARGET_DIR = "target/llvm-cov-cyberkindred"
    & cargo "+$coverageToolchain" llvm-cov -p cyberkindred --all-features --branch --no-rustc-wrapper --json --output-path "coverage/rust-cyberkindred.json"
    if ($LASTEXITCODE -ne 0) {
        throw "cyberkindred Rust coverage failed with exit code $LASTEXITCODE"
    }

    $env:CARGO_LLVM_COV_TARGET_DIR = "target/llvm-cov-windows-credential"
    & cargo "+$coverageToolchain" llvm-cov -p cyberkindred-windows-credential --branch --no-rustc-wrapper --json --output-path "coverage/rust-windows-credential.json"
    if ($LASTEXITCODE -ne 0) {
        throw "windows credential Rust coverage failed with exit code $LASTEXITCODE"
    }
}
finally {
    if ($null -eq $originalTarget) {
        Remove-Item Env:CARGO_LLVM_COV_TARGET_DIR -ErrorAction SilentlyContinue
    }
    else {
        $env:CARGO_LLVM_COV_TARGET_DIR = $originalTarget
    }
    if ($null -eq $originalTestThreads) {
        Remove-Item Env:RUST_TEST_THREADS -ErrorAction SilentlyContinue
    }
    else {
        $env:RUST_TEST_THREADS = $originalTestThreads
    }
    Pop-Location
}
