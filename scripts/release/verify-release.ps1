[CmdletBinding()]
param(
    [switch]$Development,
    [switch]$SkipTests,
    [string]$CandidatePath,
    [string]$TrustedManifestSha256
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

function Read-TrustedManifest {
    param([string]$Path, [string]$TrustedSha256)
    if ($TrustedSha256 -cnotmatch "^[a-fA-F0-9]{64}$") {
        throw "a 64-character out-of-band TrustedManifestSha256 is required"
    }
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        $actual = Get-StreamSha256 $stream
        if ($actual -cne $TrustedSha256.ToLowerInvariant()) {
            throw "candidate manifest differs from the trusted out-of-band digest"
        }
        $reader = [IO.StreamReader]::new($stream, [Text.Encoding]::UTF8, $true, 4096, $true)
        try {
            return $reader.ReadToEnd() | ConvertFrom-Json
        }
        finally {
            $reader.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
}

function Assert-UnsignedExecutableSnapshot {
    param([System.IO.Stream]$Source, [string]$FileName, [string]$ExpectedSha256)
    $temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    $stagingDirectory = [IO.Path]::GetFullPath((Join-Path $temporaryRoot ("CyberKindredSignatureVerification-" + [guid]::NewGuid().ToString("N"))))
    if (-not $stagingDirectory.StartsWith($temporaryRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw "signature staging directory escaped the temporary root"
    }
    $stagedLock = $null
    try {
        New-Item -ItemType Directory -Path $stagingDirectory | Out-Null
        $stagedPath = Join-Path $stagingDirectory $FileName
        $destination = [IO.File]::Open($stagedPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        try {
            $Source.Position = 0
            $Source.CopyTo($destination)
            $destination.Flush($true)
        }
        finally {
            $destination.Dispose()
        }
        $stagedLock = [IO.File]::Open($stagedPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        if ((Get-StreamSha256 $stagedLock) -cne $ExpectedSha256) {
            throw "signature snapshot hash differs from the trusted artifact"
        }
        $signature = Get-AuthenticodeSignature -LiteralPath $stagedPath
        if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::NotSigned) {
            throw "unsigned beta executable has unexpected signature status: $FileName ($($signature.Status))"
        }
    }
    finally {
        if ($null -ne $stagedLock) { $stagedLock.Dispose() }
        if (Test-Path -LiteralPath $stagingDirectory -PathType Container) {
            Remove-Item -LiteralPath $stagingDirectory -Recurse -Force
        }
    }
}

function Assert-CandidateManifest {
    param([string]$Path, [string]$TrustedSha256)
    $resolved = [IO.Path]::GetFullPath($Path)
    if ($resolved.StartsWith("\\", [StringComparison]::Ordinal)) {
        throw "candidate verification requires a local path"
    }
    $rootItem = Get-Item -Force -LiteralPath $resolved
    if (-not $rootItem.PSIsContainer -or $rootItem.LinkType) {
        throw "candidate root must be a real local directory"
    }
    $reparseEntries = @(
        Get-ChildItem -Force -Recurse -LiteralPath $resolved |
            Where-Object { ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 }
    )
    if ($reparseEntries.Count -gt 0) {
        throw "candidate contains a reparse point"
    }
    $manifestPath = Join-Path $resolved "manifest.json"
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw "candidate manifest is missing: $manifestPath"
    }
    $manifest = Read-TrustedManifest $manifestPath $TrustedSha256
    $package = Get-Content -Raw -LiteralPath (Join-Path $workspaceRoot "package.json") | ConvertFrom-Json
    $tauri = Get-Content -Raw -LiteralPath (Join-Path $workspaceRoot "src-tauri\tauri.conf.json") | ConvertFrom-Json
    $head = (& git -C $workspaceRoot rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw "git rev-parse failed" }
    if ($manifest.commit -cne $head) { throw "candidate commit is not the current HEAD" }
    if ($manifest.releaseEligible -ne $true -or $manifest.development -ne $false -or $manifest.dirty -ne $false) {
        throw "development or dirty artifacts cannot pass release verification"
    }
    if ($manifest.version -cne $package.version -or $manifest.version -cne $tauri.version) {
        throw "candidate version differs from package/Tauri version"
    }
    if ($manifest.unsigned -ne $true) { throw "candidate must be explicitly recorded as unsigned" }
    if ($manifest.webView2InstallMode -ne "embedBootstrapper") {
        throw "candidate manifest does not record embedBootstrapper"
    }
    if ($manifest.installerPolicy.installMode -ne "currentUser" -or
        $manifest.installerPolicy.allowDowngrades -ne $false) {
        throw "candidate manifest does not bind the required installer policy"
    }
    foreach ($config in $manifest.configuration) {
        $configPath = [IO.Path]::GetFullPath((Join-Path $workspaceRoot $config.file))
        $snapshotPath = [IO.Path]::GetFullPath((Join-Path $resolved $config.snapshot))
        if (-not $configPath.StartsWith($workspaceRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
            throw "configuration escapes workspace: $($config.file)"
        }
        if (-not $snapshotPath.StartsWith($resolved + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
            throw "configuration snapshot escapes candidate: $($config.snapshot)"
        }
        if (-not (Test-Path -LiteralPath $configPath -PathType Leaf)) {
            throw "configuration input is missing: $($config.file)"
        }
        if (-not (Test-Path -LiteralPath $snapshotPath -PathType Leaf)) {
            throw "configuration snapshot is missing: $($config.snapshot)"
        }
        $actualConfigHash = Get-LockedFileSha256 $configPath
        if ($actualConfigHash -ne $config.sha256) { throw "configuration hash differs: $($config.file)" }
        $snapshotHash = Get-LockedFileSha256 $snapshotPath
        if ($snapshotHash -ne $config.sha256) { throw "configuration snapshot hash differs: $($config.snapshot)" }
    }
    $configurationFiles = @($manifest.configuration | ForEach-Object { $_.file })
    if ($manifest.releaseTest -eq $true) {
        if ($manifest.identifier -cne "com.cyberkindred.release-test" -or
            $manifest.productName -cne "CyberKindred Release Test" -or
            $configurationFiles.Count -ne 2 -or
            $configurationFiles -notcontains "src-tauri/tauri.conf.json" -or
            $configurationFiles -notcontains "scripts/release/tauri.release-test.conf.json") {
            throw "release-test candidate identity/configuration binding is invalid"
        }
    } elseif ($manifest.identifier -cne $tauri.identifier -or
        $manifest.productName -cne $tauri.productName -or
        $configurationFiles.Count -ne 1 -or
        $configurationFiles[0] -cne "src-tauri/tauri.conf.json") {
        throw "production candidate identity differs from Tauri config"
    }
    $manifestArtifactFiles = @($manifest.artifacts | ForEach-Object { [string]$_.file } | Sort-Object -CaseSensitive)
    if ($manifestArtifactFiles.Count -ne @($manifestArtifactFiles | Select-Object -Unique).Count) {
        throw "artifact manifest contains duplicate file names"
    }
    $actualArtifactFiles = @(
        Get-ChildItem -Force -Recurse -LiteralPath $resolved -File |
            ForEach-Object { (($_.FullName.Substring($resolved.Length)) -replace '^[\\/]+', '').Replace("\", "/") } |
            Where-Object { $_ -cne "manifest.json" } |
            Sort-Object -CaseSensitive
    )
    $inventoryDifference = @(
        Compare-Object -CaseSensitive -ReferenceObject $manifestArtifactFiles -DifferenceObject $actualArtifactFiles
    )
    if ($inventoryDifference.Count -gt 0) {
        throw "candidate file inventory differs from the trusted manifest"
    }
    foreach ($artifact in $manifest.artifacts) {
        $artifactPath = [IO.Path]::GetFullPath((Join-Path $resolved $artifact.file))
        if (-not $artifactPath.StartsWith($resolved + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
            throw "artifact escapes candidate directory: $($artifact.file)"
        }
        if (-not (Test-Path -LiteralPath $artifactPath -PathType Leaf)) {
            throw "manifest artifact is missing: $($artifact.file)"
        }
        $artifactStream = [IO.File]::Open($artifactPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        try {
            $actual = Get-StreamSha256 $artifactStream
            if ($actual -ne $artifact.sha256) { throw "artifact hash differs: $($artifact.file)" }
            if ([IO.Path]::GetExtension($artifactPath) -ieq ".exe") {
                Assert-UnsignedExecutableSnapshot $artifactStream (Split-Path -Leaf $artifactPath) $artifact.sha256
            }
        }
        finally {
            $artifactStream.Dispose()
        }
    }
    $manifestInstaller = @($manifest.artifacts | Where-Object { $_.file -eq $manifest.installer.file })
    if ($manifestInstaller.Count -ne 1 -or $manifestInstaller[0].sha256 -ne $manifest.installer.sha256) {
        throw "installer identity does not match the artifact inventory"
    }
    Write-Output "Candidate artifact hashes verified: $($manifest.artifacts.Count)"
}

Push-Location $workspaceRoot
try {
    Invoke-Checked "node" @("scripts/release/generate-release-assets.mjs")
    Invoke-Checked "node" @("scripts/release/verify-release-assets.mjs")
    $preflightArguments = @("scripts/release/preflight.mjs")
    if ($Development) { $preflightArguments += "--development" }
    Invoke-Checked "node" $preflightArguments
    Invoke-Checked "cargo" @("audit", "--file", "Cargo.lock")
    Invoke-Checked "cargo" @("deny", "--all-features", "check")
    Invoke-Checked "pnpm" @("audit", "--audit-level", "high")
    if ($SkipTests) {
        Invoke-Checked "pnpm" @("build")
    } else {
        Invoke-Checked "pnpm" @("check")
        # The suite creates many isolated SQLite databases. Serial execution keeps
        # fixture writers from saturating the release host and becoming nondeterministic.
        Invoke-Checked "cargo" @("test", "--workspace", "--all-features", "--locked", "--", "--test-threads=1")
    }
    Invoke-Checked "node" $preflightArguments
    if ($CandidatePath) { Assert-CandidateManifest $CandidatePath $TrustedManifestSha256 }
    if ($Development) {
        Write-Output "Development verification completed; no candidate was release-approved."
    } else {
        Write-Output "Release verification completed."
    }
} finally {
    Pop-Location
}
