[CmdletBinding()]
param(
    [ValidateSet("x86_64-pc-windows-msvc")]
    [string]$Target = "x86_64-pc-windows-msvc"
)

$ErrorActionPreference = "Stop"
$workspaceRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))

function Invoke-Checked {
    param([string]$Command, [string[]]$Arguments, [string]$WorkingDirectory)
    Push-Location $WorkingDirectory
    try {
        & $Command @Arguments
        if ($LASTEXITCODE -ne 0) { throw "$Command failed with exit code $LASTEXITCODE" }
    } finally {
        Pop-Location
    }
}

function Assert-SafeReproWorktree {
    param([string]$Worktree, [bool]$RequireRegistered = $true)

    $fullPath = [IO.Path]::GetFullPath($Worktree)
    $expectedPrefix = Join-Path $tempRoot "cyberkindred-repro-"
    if (-not $fullPath.StartsWith($expectedPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "refusing unsafe worktree path: $fullPath"
    }
    if (Test-Path -LiteralPath $fullPath) {
        $item = Get-Item -Force -LiteralPath $fullPath
        if (-not $item.PSIsContainer -or $item.LinkType) {
            throw "refusing cleanup of a non-directory or reparse-point worktree: $fullPath"
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
            throw "refusing cleanup because the path is not the registered repro worktree: $fullPath"
        }
    }
    return $fullPath
}

function Remove-ReproWorktree {
    param([string]$Worktree)

    $fullPath = Assert-SafeReproWorktree $Worktree
    # Git for Windows needs long-path handling for pnpm's content-addressed tree.
    # The worktree assertion above makes this destructive cleanup path-specific.
    Invoke-Checked "git" @("-c", "core.longpaths=true", "-C", $fullPath, "clean", "-ffdx") $workspaceRoot
    Invoke-Checked "git" @("-c", "core.longpaths=true", "worktree", "remove", "--force", $fullPath) $workspaceRoot
}

Push-Location $workspaceRoot
try {
    if ((& git status --porcelain=v1 --untracked-files=all).Count -gt 0) {
        throw "reproducibility verification requires a clean checkout"
    }
    $commit = (& git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw "git rev-parse failed" }
} finally {
    Pop-Location
}

$tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$worktrees = @(
    (Join-Path $tempRoot ("cyberkindred-repro-a-" + [Guid]::NewGuid().ToString("N"))),
    (Join-Path $tempRoot ("cyberkindred-repro-b-" + [Guid]::NewGuid().ToString("N")))
)

try {
    foreach ($worktree in $worktrees) {
        Assert-SafeReproWorktree $worktree $false | Out-Null
        Invoke-Checked "git" @("worktree", "add", "--detach", $worktree, $commit) $workspaceRoot
        Invoke-Checked "pnpm" @("install", "--frozen-lockfile") $worktree
        $output = Join-Path $worktree "target\release-artifacts\repro"
        Invoke-Checked "powershell" @(
            "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
            (Join-Path $worktree "scripts\release\build-beta.ps1"),
            "-SnapshotBuild", "-OutputRoot", $output, "-Target", $Target
        ) $worktree
    }

    $manifests = $worktrees | ForEach-Object {
        Get-Content -Raw -LiteralPath (Join-Path $_ "target\release-artifacts\repro\manifest.json") |
            ConvertFrom-Json
    }
    foreach ($field in @("commit", "version", "target", "schemaDatabaseVersion", "identifier", "productName", "releaseEligible")) {
        if ($manifests[0].$field -cne $manifests[1].$field) {
            throw "reproducibility manifest metadata differs: $field"
        }
    }
    $first = @($manifests[0].artifacts | ForEach-Object { "$($_.file)`t$($_.bytes)`t$($_.sha256)" } | Sort-Object)
    $second = @($manifests[1].artifacts | ForEach-Object { "$($_.file)`t$($_.bytes)`t$($_.sha256)" } | Sort-Object)
    $firstBuildInputs = @($manifests[0].buildInputs | ForEach-Object { "$($_.path)`t$($_.bytes)`t$($_.sha256)" } | Sort-Object)
    $secondBuildInputs = @($manifests[1].buildInputs | ForEach-Object { "$($_.path)`t$($_.bytes)`t$($_.sha256)" } | Sort-Object)
    $differences = @(
        Compare-Object -CaseSensitive -ReferenceObject $first -DifferenceObject $second
        Compare-Object -CaseSensitive -ReferenceObject $firstBuildInputs -DifferenceObject $secondBuildInputs
    )
    if ($differences.Count -gt 0) {
        throw "reproducibility mismatch: artifact or build-input inventories differ"
    }
    Write-Output "Reproducible artifact and build-input hashes verified for commit $commit ($($first.Count) artifacts, $($firstBuildInputs.Count) build inputs)."
} finally {
    foreach ($worktree in $worktrees) {
        if (Test-Path -LiteralPath $worktree) {
            try {
                Remove-ReproWorktree $worktree
            } catch {
                Write-Warning "Repro worktree cleanup failed for $worktree`: $($_.Exception.Message)"
            }
        }
    }
}
