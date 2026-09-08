[CmdletBinding()]
param(
    [switch]$Development,
    [switch]$ReleaseTest,
    [string]$OutputRoot,
    [string]$Target = "x86_64-pc-windows-msvc"
)

$ErrorActionPreference = "Stop"
$workspaceRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))

function Invoke-Checked {
    param([string]$Command, [string[]]$Arguments)
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Command failed with exit code $LASTEXITCODE"
    }
}

Push-Location $workspaceRoot
try {
    $package = Get-Content -Raw -LiteralPath "package.json" | ConvertFrom-Json
    $shortCommit = (& git rev-parse --short=12 HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw "git rev-parse failed" }
    $env:SOURCE_DATE_EPOCH = (& git show -s --format=%ct HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw "git show failed" }

    Invoke-Checked "node" @("scripts/release/generate-release-assets.mjs")
    Invoke-Checked "node" @("scripts/release/verify-release-assets.mjs")
    $preflightArguments = @("scripts/release/preflight.mjs")
    if ($Development) { $preflightArguments += "--development" }
    Invoke-Checked "node" $preflightArguments
    Invoke-Checked "pnpm" @("install", "--frozen-lockfile")

    $buildStarted = Get-Date
    $tauriArguments = @(
        "tauri", "build", "--target", $Target, "--bundles", "nsis", "--ci", "--no-sign"
    )
    $manifestArguments = @()
    if ($ReleaseTest) {
        $releaseTestConfig = "scripts/release/tauri.release-test.conf.json"
        $tauriArguments += @("--config", $releaseTestConfig)
        $manifestArguments += @("--config", $releaseTestConfig)
    }
    Invoke-Checked "pnpm" $tauriArguments

    $releaseRoot = Join-Path $workspaceRoot "target\$Target\release"
    $application = Join-Path $releaseRoot "cyberkindred.exe"
    if (-not (Test-Path -LiteralPath $application -PathType Leaf)) {
        throw "built application is missing: $application"
    }
    $bundleRoot = Join-Path $releaseRoot "bundle\nsis"
    $installers = @(
        Get-ChildItem -LiteralPath $bundleRoot -Filter "*.exe" -File |
            Where-Object { $_.LastWriteTime -ge $buildStarted.AddSeconds(-5) }
    )
    if ($installers.Count -ne 1) {
        throw "expected exactly one freshly built NSIS installer, found $($installers.Count)"
    }

    if (-not $OutputRoot) {
        $directoryName = [string]$package.version
        if ($ReleaseTest) { $directoryName = "$directoryName-release-test-$shortCommit" }
        if ($Development) { $directoryName = "$directoryName-development" }
        $OutputRoot = Join-Path $workspaceRoot "target\release-artifacts\$directoryName"
    }
    $OutputRoot = [IO.Path]::GetFullPath($OutputRoot)
    $allowedRoot = [IO.Path]::GetFullPath((Join-Path $workspaceRoot "target\release-artifacts"))
    if (-not $OutputRoot.StartsWith($allowedRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw "release output must be below $allowedRoot"
    }
    if (Test-Path -LiteralPath $OutputRoot) {
        if (@(Get-ChildItem -Force -LiteralPath $OutputRoot).Count -gt 0) {
            throw "release output already exists and is not empty: $OutputRoot"
        }
    } else {
        New-Item -ItemType Directory -Path $OutputRoot | Out-Null
    }

    Copy-Item -LiteralPath $application -Destination $OutputRoot
    Copy-Item -LiteralPath $installers[0].FullName -Destination $OutputRoot
    foreach ($releaseAsset in @(
        "src-tauri/resources/licenses/cyberkindred.cdx.json",
        "src-tauri/resources/licenses/license-manifest.json",
        "src-tauri/resources/licenses/THIRD-PARTY-NOTICES.txt"
    )) {
        Copy-Item -LiteralPath (Join-Path $workspaceRoot $releaseAsset) -Destination $OutputRoot
    }
    $buildInputRoot = Join-Path $OutputRoot "build-inputs"
    New-Item -ItemType Directory -Path $buildInputRoot | Out-Null
    Copy-Item -LiteralPath (Join-Path $workspaceRoot "src-tauri\tauri.conf.json") `
        -Destination (Join-Path $buildInputRoot "tauri.conf.json")
    if ($ReleaseTest) {
        Copy-Item -LiteralPath (Join-Path $workspaceRoot "scripts\release\tauri.release-test.conf.json") `
            -Destination (Join-Path $buildInputRoot "tauri.release-test.conf.json")
    }
    $pdb = Join-Path $releaseRoot "cyberkindred.pdb"
    if (Test-Path -LiteralPath $pdb -PathType Leaf) {
        Copy-Item -LiteralPath $pdb -Destination $OutputRoot
    }

    Invoke-Checked "node" (@(
        "scripts/release/artifact-manifest.mjs", "--input", $OutputRoot,
        "--output", (Join-Path $OutputRoot "manifest.json")
    ) + $manifestArguments)
    Write-Output "Local unsigned beta candidate: $OutputRoot"
    if ($Development) {
        Write-Warning "This candidate came from a dirty development tree and is not release-eligible."
    }
} finally {
    Pop-Location
}
