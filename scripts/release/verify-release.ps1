[CmdletBinding()]
param(
    [switch]$Development,
    [switch]$SkipTests,
    [string]$CandidatePath
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

function Assert-CandidateManifest {
    param([string]$Path)
    $resolved = [IO.Path]::GetFullPath($Path)
    $manifestPath = Join-Path $resolved "manifest.json"
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw "candidate manifest is missing: $manifestPath"
    }
    $manifest = Get-Content -Raw -LiteralPath $manifestPath | ConvertFrom-Json
    $package = Get-Content -Raw -LiteralPath (Join-Path $workspaceRoot "package.json") | ConvertFrom-Json
    $tauri = Get-Content -Raw -LiteralPath (Join-Path $workspaceRoot "src-tauri\tauri.conf.json") | ConvertFrom-Json
    $head = (& git -C $workspaceRoot rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw "git rev-parse failed" }
    if ($manifest.commit -cne $head) { throw "candidate commit is not the current HEAD" }
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
        $actualConfigHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $configPath).Hash.ToLowerInvariant()
        if ($actualConfigHash -ne $config.sha256) { throw "configuration hash differs: $($config.file)" }
        $snapshotHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $snapshotPath).Hash.ToLowerInvariant()
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
    foreach ($artifact in $manifest.artifacts) {
        $artifactPath = [IO.Path]::GetFullPath((Join-Path $resolved $artifact.file))
        if (-not $artifactPath.StartsWith($resolved + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
            throw "artifact escapes candidate directory: $($artifact.file)"
        }
        if (-not (Test-Path -LiteralPath $artifactPath -PathType Leaf)) {
            throw "manifest artifact is missing: $($artifact.file)"
        }
        $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $artifactPath).Hash.ToLowerInvariant()
        if ($actual -ne $artifact.sha256) { throw "artifact hash differs: $($artifact.file)" }
        if ([IO.Path]::GetExtension($artifactPath) -ieq ".exe") {
            $signature = Get-AuthenticodeSignature -LiteralPath $artifactPath
            if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::NotSigned) {
                throw "unsigned beta executable has unexpected signature status: $($artifact.file) ($($signature.Status))"
            }
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
    Invoke-Checked "pnpm" @("audit", "--prod", "--audit-level", "high")
    if ($SkipTests) {
        Invoke-Checked "pnpm" @("build")
    } else {
        Invoke-Checked "pnpm" @("check")
        Invoke-Checked "cargo" @("test", "--workspace", "--all-features", "--locked")
    }
    if ($CandidatePath) { Assert-CandidateManifest $CandidatePath }
    Write-Output "Release verification completed."
} finally {
    Pop-Location
}
