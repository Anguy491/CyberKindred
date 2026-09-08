[CmdletBinding()]
param(
    [ValidateSet("Plan", "Install", "Upgrade", "RollbackGuard", "Uninstall")]
    [string]$Operation = "Plan",
    [Parameter(Mandatory = $true)]
    [string]$CandidatePath,
    [string]$TrustedManifestSha256,
    [string]$PreviousCandidatePath,
    [string]$PreviousTrustedManifestSha256,
    [string]$UninstallerPath,
    [string]$TrustedUninstallerSha256,
    [string]$EvidencePath,
    [switch]$Execute
)

$ErrorActionPreference = "Stop"
$workspaceRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$sentinelValue = "CYBERKINDRED_DISPOSABLE_VM_V1"
$sentinelRegistryPath = "HKLM:\SOFTWARE\CyberKindred\TestEnvironment"
$releaseTestIdentifier = "com.cyberkindred.release-test"
$releaseTestProduct = "CyberKindred Release Test"
$sentinelMarker = Get-ItemPropertyValue -LiteralPath $sentinelRegistryPath -Name "Marker" -ErrorAction SilentlyContinue
$hasDisposableSentinel = $sentinelMarker -ceq $sentinelValue

function Get-FileSha256 {
    param([string]$Path)
    return (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
}

function Get-StreamSha256 {
    param([System.IO.Stream]$Stream)
    $hasher = [Security.Cryptography.SHA256]::Create()
    try {
        $Stream.Position = 0
        $digest = $hasher.ComputeHash($Stream)
        $Stream.Position = 0
        return [Convert]::ToHexString($digest).ToLowerInvariant()
    }
    finally {
        $hasher.Dispose()
    }
}

function Read-ManifestSnapshot {
    param([string]$Path, [string]$TrustedSha256, [string]$Label, [bool]$RequireTrusted)
    if ($RequireTrusted -and $TrustedSha256 -cnotmatch "^[a-fA-F0-9]{64}$") {
        throw "$Label requires a 64-character out-of-band trusted manifest digest"
    }
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        $manifestHash = Get-StreamSha256 $stream
        if ($RequireTrusted -and $manifestHash -cne $TrustedSha256.ToLowerInvariant()) {
            throw "$Label manifest differs from the trusted out-of-band digest"
        }
        $reader = [IO.StreamReader]::new($stream, [Text.Encoding]::UTF8, $true, 4096, $true)
        try {
            $manifest = $reader.ReadToEnd() | ConvertFrom-Json
        }
        finally {
            $reader.Dispose()
        }
        return [ordered]@{ manifest = $manifest; sha256 = $manifestHash }
    }
    finally {
        $stream.Dispose()
    }
}

function Get-VerifiedCandidate {
    param([string]$Path, [string]$Label, [bool]$RequireCurrent, [string]$TrustedSha256, [bool]$RequireTrusted)
    if (-not $Path) { throw "$Label is required" }
    $root = [IO.Path]::GetFullPath((Resolve-Path -LiteralPath $Path).Path)
    $manifestPath = Join-Path $root "manifest.json"
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw "$Label manifest is missing"
    }
    $manifestSnapshot = Read-ManifestSnapshot $manifestPath $TrustedSha256 $Label $RequireTrusted
    $manifest = $manifestSnapshot.manifest
    $package = Get-Content -Raw -LiteralPath (Join-Path $workspaceRoot "package.json") | ConvertFrom-Json
    $tauri = Get-Content -Raw -LiteralPath (Join-Path $workspaceRoot "src-tauri\tauri.conf.json") | ConvertFrom-Json
    if ($RequireCurrent) {
        $head = (& git -C $workspaceRoot rev-parse HEAD).Trim()
        if ($LASTEXITCODE -ne 0) { throw "git rev-parse failed" }
        if ($manifest.commit -cne $head) { throw "$Label commit is not the current HEAD" }
        if ($manifest.version -cne $package.version -or $manifest.version -cne $tauri.version) {
            throw "$Label version differs from package/Tauri version"
        }
    }
    if ($manifest.unsigned -ne $true -or $manifest.webView2InstallMode -ne "embedBootstrapper") {
        throw "$Label is not an unsigned embedBootstrapper candidate"
    }
    if ($manifest.releaseEligible -ne $true -or $manifest.development -ne $false -or $manifest.dirty -ne $false) {
        throw "$Label is development, dirty, or not release eligible"
    }
    if ($manifest.installerPolicy.installMode -ne "currentUser" -or
        $manifest.installerPolicy.allowDowngrades -ne $false) {
        throw "$Label does not bind the current-user/no-downgrade policy"
    }
    foreach ($config in $manifest.configuration) {
        $configPath = [IO.Path]::GetFullPath((Join-Path $workspaceRoot $config.file))
        $snapshotPath = [IO.Path]::GetFullPath((Join-Path $root $config.snapshot))
        if (-not $configPath.StartsWith($workspaceRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
            throw "$Label configuration escapes the workspace"
        }
        if (-not $snapshotPath.StartsWith($root + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
            throw "$Label configuration snapshot escapes the candidate"
        }
        if (-not (Test-Path -LiteralPath $snapshotPath -PathType Leaf)) {
            throw "$Label configuration snapshot is missing: $($config.snapshot)"
        }
        if ((Get-FileSha256 $snapshotPath) -ne $config.sha256) {
            throw "$Label configuration snapshot hash differs: $($config.snapshot)"
        }
        if ($RequireCurrent) {
            if (-not (Test-Path -LiteralPath $configPath -PathType Leaf)) {
                throw "$Label configuration is missing: $($config.file)"
            }
            if ((Get-FileSha256 $configPath) -ne $config.sha256) {
                throw "$Label configuration hash differs: $($config.file)"
            }
        }
    }
    $installerPath = [IO.Path]::GetFullPath((Join-Path $root $manifest.installer.file))
    if (-not $installerPath.StartsWith($root + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Label installer escapes the candidate directory"
    }
    if (-not (Test-Path -LiteralPath $installerPath -PathType Leaf)) {
        throw "$Label installer is missing"
    }
    $installerHash = Get-FileSha256 $installerPath
    $artifactMatches = @(
        $manifest.artifacts |
            Where-Object { $_.file -eq $manifest.installer.file -and $_.sha256 -eq $installerHash }
    )
    if ($installerHash -ne $manifest.installer.sha256 -or $artifactMatches.Count -ne 1) {
        throw "$Label installer hash is not bound to its artifact manifest"
    }
    $releaseTestConfig = @(
        $manifest.configuration |
            Where-Object { $_.file -eq "scripts/release/tauri.release-test.conf.json" }
    )
    $baseConfig = @(
        $manifest.configuration |
            Where-Object { $_.file -eq "src-tauri/tauri.conf.json" }
    )
    $isReleaseTest = (
        $manifest.releaseTest -eq $true -and
        $manifest.identifier -ceq $releaseTestIdentifier -and
        $manifest.productName -ceq $releaseTestProduct -and
        $manifest.configuration.Count -eq 2 -and
        $releaseTestConfig.Count -eq 1 -and
        $baseConfig.Count -eq 1
    )
    if (-not $isReleaseTest -and (
        $manifest.identifier -cne $tauri.identifier -or
        $manifest.productName -cne $tauri.productName -or
        $manifest.configuration.Count -ne 1 -or
        $baseConfig.Count -ne 1
    )) {
        throw "$Label production identity/configuration binding is invalid"
    }
    return [ordered]@{
        root = $root
        manifest = $manifest
        installerPath = $installerPath
        installerHash = $installerHash
        releaseTest = $isReleaseTest
        manifestHash = $manifestSnapshot.sha256
    }
}

function Invoke-Installer {
    param([string]$Path, [string]$ExpectedSha256, [int[]]$ExpectedExitCodes)
    if ($ExpectedSha256 -cnotmatch "^[a-fA-F0-9]{64}$") {
        throw "installer execution requires a manifest-bound SHA-256"
    }
    $source = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    $temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
    $stagingDirectory = [IO.Path]::GetFullPath((Join-Path $temporaryRoot ("CyberKindredInstallerVerification-" + [guid]::NewGuid().ToString("N"))))
    if (-not $stagingDirectory.StartsWith($temporaryRoot, [StringComparison]::OrdinalIgnoreCase)) {
        $source.Dispose()
        throw "installer staging directory escaped the temporary root"
    }
    $stagedLock = $null
    try {
        $sourceHash = Get-StreamSha256 $source
        if ($sourceHash -cne $ExpectedSha256.ToLowerInvariant()) {
            throw "installer differs from its trusted manifest immediately before execution"
        }
        New-Item -ItemType Directory -Path $stagingDirectory | Out-Null
        $stagedPath = Join-Path $stagingDirectory (Split-Path -Leaf $Path)
        $destination = [IO.File]::Open($stagedPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        try {
            $source.CopyTo($destination)
            $destination.Flush($true)
        }
        finally {
            $destination.Dispose()
        }
        $stagedLock = [IO.File]::Open($stagedPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        if ((Get-StreamSha256 $stagedLock) -cne $ExpectedSha256.ToLowerInvariant()) {
            throw "staged installer hash differs before execution"
        }
        $process = Start-Process -FilePath $stagedPath -ArgumentList "/S" -Wait -PassThru -WindowStyle Hidden
        if ($ExpectedExitCodes -notcontains $process.ExitCode) {
            throw "installer $(Split-Path -Leaf $Path) exited $($process.ExitCode); expected $($ExpectedExitCodes -join ', ')"
        }
        return [ordered]@{
            file = Split-Path -Leaf $Path
            sha256 = $ExpectedSha256.ToLowerInvariant()
            exitCode = $process.ExitCode
        }
    }
    finally {
        if ($null -ne $stagedLock) { $stagedLock.Dispose() }
        $source.Dispose()
        if (Test-Path -LiteralPath $stagingDirectory -PathType Container) {
            Remove-Item -LiteralPath $stagingDirectory -Recurse -Force
        }
    }
}

$candidate = Get-VerifiedCandidate $CandidatePath "CandidatePath" $true $TrustedManifestSha256 ([bool]$Execute)
$previousCandidate = $null
if ($Operation -in @("Upgrade", "RollbackGuard")) {
    $previousCandidate = Get-VerifiedCandidate $PreviousCandidatePath "PreviousCandidatePath" $false $PreviousTrustedManifestSha256 ([bool]$Execute)
    if ($previousCandidate.manifest.identifier -cne $candidate.manifest.identifier) {
        throw "current and previous candidates use different identifiers"
    }
    if ([version]$previousCandidate.manifest.version -ge [version]$candidate.manifest.version) {
        throw "previous candidate version must be lower than the current candidate version"
    }
}
if ($Operation -ne "Plan" -and -not $Execute) {
    throw "mutating installer operations require the explicit -Execute switch"
}
if ($Execute -and -not ($candidate.releaseTest -or $hasDisposableSentinel)) {
    throw "refusing installer mutation: production candidates require the protected machine-wide disposable-VM marker"
}
if ($Execute -and $previousCandidate -and -not ($previousCandidate.releaseTest -or $hasDisposableSentinel)) {
    throw "refusing previous installer mutation: production candidates require the protected machine-wide disposable-VM marker"
}
$steps = @()
if ($Operation -eq "Plan") {
    $steps += [ordered]@{
        action = "plan"
        installer = Split-Path -Leaf $candidate.installerPath
        sha256 = $candidate.installerHash
        executed = $false
    }
} elseif ($Operation -eq "Install") {
    $steps += Invoke-Installer $candidate.installerPath $candidate.installerHash @(0)
} elseif ($Operation -eq "Upgrade") {
    $steps += Invoke-Installer $previousCandidate.installerPath $previousCandidate.installerHash @(0)
    $steps += Invoke-Installer $candidate.installerPath $candidate.installerHash @(0)
} elseif ($Operation -eq "RollbackGuard") {
    $steps += Invoke-Installer $candidate.installerPath $candidate.installerHash @(0)
    $downgrade = Invoke-Installer $previousCandidate.installerPath $previousCandidate.installerHash @(1)
    $installedRoot = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA $candidate.manifest.productName))
    $installedApplication = Join-Path $installedRoot "cyberkindred.exe"
    $candidateApplication = @($candidate.manifest.artifacts | Where-Object { $_.file -ceq "cyberkindred.exe" })
    if ($candidateApplication.Count -ne 1 -or
        -not (Test-Path -LiteralPath $installedApplication -PathType Leaf) -or
        (Get-FileSha256 $installedApplication) -cne $candidateApplication[0].sha256) {
        throw "downgrade rejection did not preserve the installed candidate executable"
    }
    $steps += [ordered]@{
        file = Split-Path -Leaf $previousCandidate.installerPath
        sha256 = $previousCandidate.installerHash
        exitCode = $downgrade.exitCode
        expected = "NSIS exit 1 with the current candidate executable preserved"
    }
} elseif ($Operation -eq "Uninstall") {
    if (-not $UninstallerPath) { throw "UninstallerPath is required" }
    $uninstaller = [IO.Path]::GetFullPath((Resolve-Path -LiteralPath $UninstallerPath).Path)
    if (-not (Test-Path -LiteralPath $uninstaller -PathType Leaf)) { throw "UninstallerPath is not a file" }
    $expectedRoot = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA $candidate.manifest.productName))
    if ((Split-Path -Leaf $uninstaller) -cne "uninstall.exe" -or
        (Split-Path -Parent $uninstaller) -cne $expectedRoot) {
        throw "uninstaller must be the exact manifest-bound LocalAppData product uninstaller"
    }
    if ($TrustedUninstallerSha256 -cnotmatch "^[a-fA-F0-9]{64}$") {
        throw "uninstaller requires the trusted post-install digest"
    }
    $steps += Invoke-Installer $uninstaller $TrustedUninstallerSha256 @(0)
}

$evidence = [ordered]@{
    schemaVersion = 1
    identifier = $candidate.manifest.identifier
    productName = $candidate.manifest.productName
    candidateManifestSha256 = $candidate.manifestHash
    operation = $Operation
    isolation = if ($candidate.releaseTest) {
        "manifest-bound-release-test"
    } elseif ($hasDisposableSentinel) {
        "disposable-vm-sentinel"
    } else {
        "verified-plan-only"
    }
    executed = [bool]($Execute -and $Operation -ne "Plan")
    steps = $steps
}
if ($EvidencePath) {
    $fullEvidencePath = [IO.Path]::GetFullPath($EvidencePath)
    $parent = Split-Path -Parent $fullEvidencePath
    if (-not (Test-Path -LiteralPath $parent -PathType Container)) {
        New-Item -ItemType Directory -Path $parent | Out-Null
    }
    $evidence | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $fullEvidencePath -Encoding UTF8
}
$evidence | ConvertTo-Json -Depth 8
