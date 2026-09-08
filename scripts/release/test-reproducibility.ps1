[CmdletBinding()]
param([string]$Target = "x86_64-pc-windows-msvc")

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
        if (-not $worktree.StartsWith((Join-Path $tempRoot "cyberkindred-repro-"), [StringComparison]::OrdinalIgnoreCase)) {
            throw "refusing unsafe worktree path: $worktree"
        }
        Invoke-Checked "git" @("worktree", "add", "--detach", $worktree, $commit) $workspaceRoot
        Invoke-Checked "pnpm" @("install", "--frozen-lockfile") $worktree
        $output = Join-Path $worktree "target\release-artifacts\repro"
        Invoke-Checked "powershell" @(
            "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
            (Join-Path $worktree "scripts\release\build-beta.ps1"),
            "-OutputRoot", $output, "-Target", $Target
        ) $worktree
    }

    $manifests = $worktrees | ForEach-Object {
        Get-Content -Raw -LiteralPath (Join-Path $_ "target\release-artifacts\repro\manifest.json") |
            ConvertFrom-Json
    }
    $first = @($manifests[0].artifacts)
    $secondByFile = @{}
    foreach ($artifact in $manifests[1].artifacts) { $secondByFile[$artifact.file] = $artifact.sha256 }
    $differences = @($first | Where-Object { $secondByFile[$_.file] -ne $_.sha256 })
    if ($differences.Count -gt 0) {
        throw "reproducibility mismatch: $($differences.file -join ', ')"
    }
    Write-Output "Reproducible artifact hashes verified for commit $commit ($($first.Count) compared files)."
} finally {
    foreach ($worktree in $worktrees) {
        if ($worktree.StartsWith((Join-Path $tempRoot "cyberkindred-repro-"), [StringComparison]::OrdinalIgnoreCase) -and
            (Test-Path -LiteralPath $worktree)) {
            & git -C $workspaceRoot worktree remove --force $worktree
        }
    }
}
