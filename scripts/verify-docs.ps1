param(
    [string]$Python = "python"
)

$ErrorActionPreference = "Stop"
$scriptDirectory = Split-Path -Parent $MyInvocation.MyCommand.Path
$repositoryRoot = Split-Path -Parent $scriptDirectory

Push-Location $repositoryRoot
try {
    & $Python ".\scripts\verify-docs.py"
    if ($LASTEXITCODE -ne 0) {
        throw "Documentation verification failed with exit code $LASTEXITCODE."
    }
}
finally {
    Pop-Location
}
