[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Revision,
    [ValidateRange(2, 2)]
    [int]$Passes = 2,
    [string]$EvidencePath = "target/m7-evidence/clean-suite.json",
    [switch]$Plan
)

$ErrorActionPreference = "Stop"
$workspaceRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "..\.."))
$temporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$worktreePrefix = "cyberkindred-clean-suite-"
$createdWorktrees = [Collections.Generic.List[string]]::new()
$results = [Collections.Generic.List[object]]::new()
$failure = $null

function Invoke-Checked {
    param(
        [Parameter(Mandatory = $true)][string]$Command,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory
    )
    Push-Location $WorkingDirectory
    try {
        & $Command @Arguments
        if ($LASTEXITCODE -ne 0) {
            throw "$Command failed with exit code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
    }
}

function Resolve-Commit {
    Push-Location $workspaceRoot
    try {
        $value = (& git rev-parse --verify "$Revision^{commit}").Trim()
        if ($LASTEXITCODE -ne 0 -or $value -notmatch '^[0-9a-f]{40}$') {
            throw "Revision does not resolve to one commit"
        }
        return $value
    } finally {
        Pop-Location
    }
}

function Assert-NoLiveEnvironment {
    $forbidden = @(
        Get-ChildItem Env: | Where-Object {
            $_.Value -and ($_.Name -match '^CYBERKINDRED_LIVE_' -or $_.Name -in @(
                'OPENAI_API_KEY', 'OPENAI_KEY', 'CYBERKINDRED_OPENAI_KEY'
            ))
        } | Select-Object -ExpandProperty Name
    )
    if ($forbidden.Count -gt 0) {
        throw "Clean suite refuses live or secret environment variables: $($forbidden -join ', ')"
    }
}

function Assert-SafeCreatedWorktree {
    param([Parameter(Mandatory = $true)][string]$Path)
    $resolved = [IO.Path]::GetFullPath($Path)
    $prefixPath = Join-Path $temporaryRoot $worktreePrefix
    if (-not $resolved.StartsWith($prefixPath, [StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetFullPath((Split-Path -Parent $resolved)) -ne $temporaryRoot -or
        -not $createdWorktrees.Contains($resolved)) {
        throw "Refusing cleanup of a path not created by this clean-suite run"
    }
    $item = Get-Item -LiteralPath $resolved -Force
    if (-not $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "Refusing cleanup of a non-directory or reparse-point worktree"
    }
    $registered = @(
        & git -C $workspaceRoot worktree list --porcelain |
            Where-Object { $_.StartsWith('worktree ', [StringComparison]::Ordinal) } |
            ForEach-Object { [IO.Path]::GetFullPath($_.Substring(9)) }
    )
    if (-not ($registered | Where-Object { $_ -eq $resolved })) {
        throw "Refusing cleanup because the created path is no longer a registered worktree"
    }
}

function Resolve-EvidencePath {
    $resolved = [IO.Path]::GetFullPath((Join-Path $workspaceRoot $EvidencePath))
    $allowed = [IO.Path]::GetFullPath((Join-Path $workspaceRoot "target\m7-evidence"))
    if ($resolved -ne $allowed -and
        -not $resolved.StartsWith($allowed + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw "EvidencePath must stay below target/m7-evidence"
    }
    return $resolved
}

Assert-NoLiveEnvironment
$commit = Resolve-Commit
$planDocument = [ordered]@{
    schemaVersion = 1
    testId = "M7-CLEAN-SUITE"
    revision = $commit
    passes = $Passes
    commands = @("pnpm install --frozen-lockfile", "pnpm verify", "git status --porcelain")
    isolation = "two detached worktrees below the process temporary directory"
}
if ($Plan) {
    $planDocument | ConvertTo-Json -Depth 6
    return
}

Push-Location $workspaceRoot
try {
    if (@(& git status --porcelain=v1 --untracked-files=all).Count -gt 0) {
        throw "Clean-suite execution requires a clean source worktree"
    }
    $package = Get-Content -Raw -LiteralPath "package.json" | ConvertFrom-Json
    if (-not $package.scripts.verify) {
        throw "package.json must define the canonical pnpm verify script before clean-suite execution"
    }
} finally {
    Pop-Location
}

try {
    for ($index = 1; $index -le $Passes; $index += 1) {
        $worktree = [IO.Path]::GetFullPath((Join-Path $temporaryRoot (
            $worktreePrefix + [Guid]::NewGuid().ToString("N")
        )))
        Invoke-Checked "git" @("worktree", "add", "--detach", $worktree, $commit) $workspaceRoot
        $createdWorktrees.Add($worktree)
        $started = [Diagnostics.Stopwatch]::StartNew()
        try {
            Invoke-Checked "pnpm" @("install", "--frozen-lockfile") $worktree
            Invoke-Checked "pnpm" @("verify") $worktree
            $dirty = @(& git -C $worktree status --porcelain=v1 --untracked-files=all)
            if ($LASTEXITCODE -ne 0) { throw "git status failed in clean pass $index" }
            if ($dirty.Count -gt 0) { throw "clean pass $index changed tracked or untracked workspace files" }
            $started.Stop()
            $results.Add([ordered]@{
                pass = $index
                status = "passed"
                durationMs = $started.ElapsedMilliseconds
            })
        } catch {
            $started.Stop()
            $results.Add([ordered]@{
                pass = $index
                status = "failed"
                durationMs = $started.ElapsedMilliseconds
                reason = "command_failed"
            })
            throw
        }
    }
} catch {
    $failure = $_
} finally {
    foreach ($worktree in $createdWorktrees) {
        try {
            Assert-SafeCreatedWorktree $worktree
            Invoke-Checked "git" @("worktree", "remove", "--force", $worktree) $workspaceRoot
        } catch {
            if ($null -eq $failure) { $failure = $_ }
        }
    }
    $evidence = [ordered]@{
        schemaVersion = 1
        testId = "M7-CLEAN-SUITE"
        revision = $commit
        requestedPasses = $Passes
        completedPasses = @($results | Where-Object { $_.status -eq 'passed' }).Count
        status = if ($null -eq $failure -and $results.Count -eq $Passes) { "passed" } else { "failed" }
        results = $results
    }
    $resolvedEvidence = Resolve-EvidencePath
    $evidenceDirectory = Split-Path -Parent $resolvedEvidence
    if (-not (Test-Path -LiteralPath $evidenceDirectory -PathType Container)) {
        New-Item -ItemType Directory -Path $evidenceDirectory | Out-Null
    }
    $evidence | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $resolvedEvidence -Encoding UTF8
}

if ($null -ne $failure) { throw $failure }
$results | Format-Table -AutoSize
