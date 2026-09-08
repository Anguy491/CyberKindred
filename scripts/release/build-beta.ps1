[CmdletBinding()]
param(
    [switch]$Development,
    [switch]$ReleaseTest,
    [switch]$SnapshotBuild,
    [string]$OutputRoot,
    [ValidateSet("x86_64-pc-windows-msvc")]
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

function Get-StreamSha256 {
    param([System.IO.Stream]$Stream)
    $hasher = [Security.Cryptography.SHA256]::Create()
    try {
        $Stream.Position = 0
        $digest = $hasher.ComputeHash($Stream)
        $Stream.Position = 0
        return ([BitConverter]::ToString($digest)).Replace("-", "").ToLowerInvariant()
    }
    finally {
        $hasher.Dispose()
    }
}

function Get-LockedFileSha256 {
    param([string]$Path)
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        return Get-StreamSha256 $stream
    }
    finally {
        $stream.Dispose()
    }
}

function Assert-ReleaseOutputPath {
    param([string]$Path, [string]$Root)
    $fullPath = [IO.Path]::GetFullPath($Path)
    $allowedRoot = [IO.Path]::GetFullPath((Join-Path $Root "target\release-artifacts"))
    if (-not $fullPath.StartsWith($allowedRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw "release output must be below $allowedRoot"
    }
    return $fullPath
}

function Assert-SafeBuildSnapshot {
    param([string]$Path, [bool]$RequireRegistered = $true)
    $fullPath = [IO.Path]::GetFullPath($Path)
    $temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    $expectedPrefix = Join-Path $temporaryRoot "cyberkindred-build-snapshot-"
    if (-not $fullPath.StartsWith($expectedPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "refusing unsafe build snapshot path: $fullPath"
    }
    if (Test-Path -LiteralPath $fullPath) {
        $item = Get-Item -Force -LiteralPath $fullPath
        if (-not $item.PSIsContainer -or $item.LinkType) {
            throw "refusing cleanup of a non-directory or reparse-point build snapshot: $fullPath"
        }
    }
    if ($RequireRegistered) {
        $registered = @(
            & git -C $workspaceRoot worktree list --porcelain |
                Where-Object { $_.StartsWith("worktree ", [StringComparison]::Ordinal) } |
                ForEach-Object { [IO.Path]::GetFullPath($_.Substring(9)) } |
                Where-Object { $_.Equals($fullPath, [StringComparison]::OrdinalIgnoreCase) }
        )
        if ($registered.Count -ne 1) {
            throw "refusing cleanup because the path is not the registered build snapshot: $fullPath"
        }
    }
    return $fullPath
}

function Assert-SafeReleaseStaging {
    param([string]$Path, [string]$AllowedRoot)
    $fullPath = [IO.Path]::GetFullPath($Path)
    $expectedPrefix = Join-Path ([IO.Path]::GetFullPath($AllowedRoot)) ".candidate-staging-"
    if (-not $fullPath.StartsWith($expectedPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "refusing unsafe candidate staging path: $fullPath"
    }
    if (Test-Path -LiteralPath $fullPath) {
        $item = Get-Item -Force -LiteralPath $fullPath
        if (-not $item.PSIsContainer -or $item.LinkType) {
            throw "candidate staging path must be a real local directory: $fullPath"
        }
    }
    return $fullPath
}

if (-not $Development -and -not $SnapshotBuild) {
    Push-Location $workspaceRoot
    try {
        Invoke-Checked "node" @("scripts/release/generate-release-assets.mjs")
        Invoke-Checked "node" @("scripts/release/verify-release-assets.mjs")
        Invoke-Checked "node" @("scripts/release/preflight.mjs")
        $sourceCommit = (& git rev-parse HEAD).Trim()
        if ($LASTEXITCODE -ne 0) { throw "git rev-parse failed" }
        if ((& git status --porcelain=v1 --untracked-files=all).Count -gt 0) {
            throw "release snapshot requires a clean checkout"
        }
        $package = Get-Content -Raw -LiteralPath "package.json" | ConvertFrom-Json
        $shortCommit = (& git rev-parse --short=12 HEAD).Trim()
        if (-not $OutputRoot) {
            $directoryName = [string]$package.version
            if ($ReleaseTest) { $directoryName = "$directoryName-release-test-$shortCommit" }
            $OutputRoot = Join-Path $workspaceRoot "target\release-artifacts\$directoryName"
        }
        $OutputRoot = Assert-ReleaseOutputPath $OutputRoot $workspaceRoot
        if (Test-Path -LiteralPath $OutputRoot) {
            throw "release output already exists: $OutputRoot"
        }
    }
    finally {
        Pop-Location
    }

    $snapshotRoot = Join-Path ([IO.Path]::GetFullPath([IO.Path]::GetTempPath())) ("cyberkindred-build-snapshot-" + [guid]::NewGuid().ToString("N"))
    $snapshotRegistered = $false
    $candidateStaging = $null
    $publishedOutput = $false
    $releaseVerified = $false
    try {
        Assert-SafeBuildSnapshot $snapshotRoot $false | Out-Null
        Invoke-Checked "git" @("-c", "core.longpaths=true", "-C", $workspaceRoot, "worktree", "add", "--detach", $snapshotRoot, $sourceCommit)
        $snapshotRegistered = $true
        Push-Location $snapshotRoot
        try {
            Invoke-Checked "pnpm" @("install", "--frozen-lockfile")
            $snapshotOutput = Join-Path $snapshotRoot "target\release-artifacts\candidate"
            $arguments = @(
                "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                (Join-Path $snapshotRoot "scripts\release\build-beta.ps1"),
                "-SnapshotBuild", "-OutputRoot", $snapshotOutput, "-Target", $Target
            )
            if ($ReleaseTest) { $arguments += "-ReleaseTest" }
            $innerOutput = @(& powershell @arguments)
            if ($LASTEXITCODE -ne 0) { throw "isolated build failed with exit code $LASTEXITCODE" }
            $innerOutput |
                Where-Object {
                    $_ -notmatch '^Local unsigned beta candidate:' -and
                    $_ -notmatch '^Trusted manifest SHA-256 \(record outside the candidate directory\):'
                } |
                ForEach-Object { Write-Output $_ }
            $digestLines = @($innerOutput | Where-Object { $_ -match '^Trusted manifest SHA-256 \(record outside the candidate directory\): ([a-f0-9]{64})$' })
            if ($digestLines.Count -ne 1) {
                throw "isolated build did not return exactly one trusted manifest digest"
            }
            $trustedManifestHash = [regex]::Match(
                [string]$digestLines[0],
                '^Trusted manifest SHA-256 \(record outside the candidate directory\): ([a-f0-9]{64})$'
            ).Groups[1].Value
        }
        finally {
            Pop-Location
        }

        Push-Location $workspaceRoot
        try {
            $finalCommit = (& git rev-parse HEAD).Trim()
            if ($LASTEXITCODE -ne 0) { throw "git rev-parse failed after build" }
            if ($finalCommit -cne $sourceCommit -or (& git status --porcelain=v1 --untracked-files=all).Count -gt 0) {
                throw "source checkout changed during the isolated build; refusing to publish the local candidate"
            }
        }
        finally {
            Pop-Location
        }

        $allowedOutputRoot = [IO.Path]::GetFullPath((Join-Path $workspaceRoot "target\release-artifacts"))
        if (-not (Test-Path -LiteralPath $allowedOutputRoot -PathType Container)) {
            New-Item -ItemType Directory -Path $allowedOutputRoot | Out-Null
        }
        $allowedRootItem = Get-Item -Force -LiteralPath $allowedOutputRoot
        if (-not $allowedRootItem.PSIsContainer -or $allowedRootItem.LinkType) {
            throw "release artifact root must be a real local directory"
        }
        $candidateStaging = Join-Path $allowedOutputRoot (".candidate-staging-" + [guid]::NewGuid().ToString("N"))
        Assert-SafeReleaseStaging $candidateStaging $allowedOutputRoot | Out-Null
        [IO.Directory]::CreateDirectory($candidateStaging) | Out-Null
        Get-ChildItem -Force -LiteralPath $snapshotOutput | ForEach-Object {
            Copy-Item -LiteralPath $_.FullName -Destination $candidateStaging -Recurse
        }
        Invoke-Checked "powershell" @(
            "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
            (Join-Path $workspaceRoot "scripts\release\verify-release.ps1"),
            "-CandidatePath", $candidateStaging, "-TrustedManifestSha256", $trustedManifestHash,
            "-SkipTests"
        )
        if (Test-Path -LiteralPath $OutputRoot) {
            throw "release output appeared during verification; refusing to overwrite it: $OutputRoot"
        }
        [IO.Directory]::Move($candidateStaging, $OutputRoot)
        $candidateStaging = $null
        $publishedOutput = $true
        Invoke-Checked "powershell" @(
            "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
            (Join-Path $workspaceRoot "scripts\release\verify-release.ps1"),
            "-CandidatePath", $OutputRoot, "-TrustedManifestSha256", $trustedManifestHash,
            "-SkipTests"
        )
        $releaseVerified = $true
        Write-Output "Local unsigned beta candidate: $OutputRoot"
        Write-Output "Trusted manifest SHA-256 (record outside the candidate directory): $trustedManifestHash"
    }
    finally {
        if ($publishedOutput -and -not $releaseVerified -and (Test-Path -LiteralPath $OutputRoot)) {
            try {
                Assert-ReleaseOutputPath $OutputRoot $workspaceRoot | Out-Null
                $failedOutput = Get-Item -Force -LiteralPath $OutputRoot
                if (-not $failedOutput.PSIsContainer -or $failedOutput.LinkType) {
                    throw "failed release output is not a real directory"
                }
                Remove-Item -LiteralPath $OutputRoot -Recurse -Force
            }
            catch {
                Write-Warning "unverified release output cleanup failed for $OutputRoot`: $($_.Exception.Message)"
            }
        }
        if ($candidateStaging -and (Test-Path -LiteralPath $candidateStaging)) {
            try {
                Assert-SafeReleaseStaging $candidateStaging (Join-Path $workspaceRoot "target\release-artifacts") | Out-Null
                Remove-Item -LiteralPath $candidateStaging -Recurse -Force
            }
            catch {
                Write-Warning "candidate staging cleanup failed for $candidateStaging`: $($_.Exception.Message)"
            }
        }
        if ($snapshotRegistered) {
            try {
                Assert-SafeBuildSnapshot $snapshotRoot | Out-Null
                Invoke-Checked "git" @("-c", "core.longpaths=true", "-C", $snapshotRoot, "clean", "-ffdx")
                Invoke-Checked "git" @("-c", "core.longpaths=true", "worktree", "remove", "--force", $snapshotRoot)
            }
            catch {
                Write-Warning "build snapshot cleanup failed for $snapshotRoot`: $($_.Exception.Message)"
            }
        }
    }
    return
}

Push-Location $workspaceRoot
try {
    $package = Get-Content -Raw -LiteralPath "package.json" | ConvertFrom-Json
    $shortCommit = (& git rev-parse --short=12 HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw "git rev-parse failed" }
    $env:SOURCE_DATE_EPOCH = (& git show -s --format=%ct HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw "git show failed" }
    $reproducibleRustFlags = @(
        "--remap-path-prefix=$workspaceRoot=/cyberkindred",
        "-C",
        "link-arg=/Brepro"
    )
    $env:CARGO_ENCODED_RUSTFLAGS = $reproducibleRustFlags -join [char]0x1f
    $env:CARGO_INCREMENTAL = "0"

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
    $manifestArguments = @("--target", $Target)
    if ($Development) { $manifestArguments += "--development" }
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
    $OutputRoot = Assert-ReleaseOutputPath $OutputRoot $workspaceRoot
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
    $trustedManifestHash = Get-LockedFileSha256 (Join-Path $OutputRoot "manifest.json")
    Write-Output "Local unsigned beta candidate: $OutputRoot"
    Write-Output "Trusted manifest SHA-256 (record outside the candidate directory): $trustedManifestHash"
    if ($Development) {
        Write-Warning "This candidate came from a dirty development tree and is not release-eligible."
    }
} finally {
    Pop-Location
}
