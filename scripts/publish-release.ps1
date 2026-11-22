# Publishes the installer built by package-windows.ps1 as a GitHub release, which installed
# copies of Miru then offer as an update.
#
# Before running: bump `version` in Cargo.toml, run scripts/package-windows.ps1, and make sure
# the GitHub CLI is signed in (`gh auth login`). The release goes to MIRU_UPDATE_REPO from
# .env, which must be a public repo (the app reads it without signing in).
#
#   .\scripts\publish-release.ps1 -Notes "What changed in this version"

param(
    [string]$Notes = ''
)

$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$repo = $null
$envFile = Join-Path $root '.env'
if (Test-Path $envFile) {
    foreach ($line in Get-Content $envFile) {
        if ($line -match '^\s*(?:export\s+)?MIRU_UPDATE_REPO\s*=\s*(.*)$') {
            $repo = $Matches[1].Trim().Trim('"').Trim("'")
        }
    }
}
if (-not $repo) { throw 'Set MIRU_UPDATE_REPO=owner/name in .env first.' }
if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    throw 'The GitHub CLI (gh) is needed: https://cli.github.com'
}

$version = (cargo pkgid -p miru) -replace '.*[#@]', ''
$installer = Join-Path $root "dist\miru_${version}_x64-setup.exe"
if (-not (Test-Path $installer)) {
    throw "No installer for version $version. Run scripts/package-windows.ps1 first."
}

gh release create "v$version" $installer --repo $repo --title "Miru $version" --notes $Notes
if ($LASTEXITCODE -ne 0) { throw 'Publishing failed.' }
Write-Host "Published Miru $version to github.com/$repo."
