# Builds a release of Miru and packages it for Windows:
#   dist/miru_<version>_x64-setup.exe  installer (per-user, no admin prompt)
#   dist/Miru-<version>-portable.zip   unzip-and-run copy
# Both include libmpv. Needs cargo-packager (`cargo install cargo-packager --locked`) and
# libmpv staged in vendor/mpv-dev (scripts/setup-mpv-windows.ps1).
#
# No TMDB key is built in: people add their own in Settings → Metadata.
# One value from .env is built into the release (it is not secret):
#   MIRU_UPDATE_REPO  public GitHub repo (owner/name) whose releases the app updates from

# Not 'Stop': Windows PowerShell treats cargo's progress output on stderr as errors when
# output is redirected. Native steps are checked through $LASTEXITCODE instead.
$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$dll = Join-Path $root 'vendor\mpv-dev\libmpv-2.dll'
if (-not (Test-Path $dll)) {
    throw 'libmpv is not staged. Run scripts/setup-mpv-windows.ps1 first.'
}
if (-not (Get-Command cargo-packager -ErrorAction SilentlyContinue)) {
    throw 'cargo-packager is missing. Install it with: cargo install cargo-packager --locked'
}

$envFile = Join-Path $root '.env'
if (Test-Path $envFile) {
    foreach ($line in Get-Content $envFile) {
        if ($line -match '^\s*(?:export\s+)?(MIRU_UPDATE_REPO)\s*=\s*(.*)$') {
            Set-Item -Path "Env:$($Matches[1])" -Value $Matches[2].Trim().Trim('"').Trim("'")
        }
    }
}
if ($env:MIRU_UPDATE_REPO) {
    Write-Host "Updates will come from github.com/$($env:MIRU_UPDATE_REPO)."
} else {
    Write-Warning 'No MIRU_UPDATE_REPO in .env: this build will not check for updates.'
}

cargo build --release -p miru
if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }

cargo packager --release -p miru
if ($LASTEXITCODE -ne 0) { throw 'Packaging failed.' }

# Portable copy: the executable and libmpv side by side.
$version = (cargo pkgid -p miru) -replace '.*[#@]', ''
$staging = Join-Path $root "dist\Miru-$version-portable"
if (Test-Path $staging) { Remove-Item -Recurse -Force $staging }
New-Item -ItemType Directory -Force $staging | Out-Null
Copy-Item (Join-Path $root 'target\release\miru.exe') $staging -ErrorAction Stop
Copy-Item $dll $staging -ErrorAction Stop
$zip = "$staging.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path (Join-Path $staging '*') -DestinationPath $zip -ErrorAction Stop
Remove-Item -Recurse -Force $staging

Write-Host "Packages are in $(Join-Path $root 'dist')"
