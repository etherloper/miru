# Builds an MSVC import library (mpv.lib) for an existing libmpv-2.dll and stages both in
# vendor/mpv-dev, which crates/player/build.rs adds to the linker search path.
# Usage: ./scripts/setup-mpv-windows.ps1 -Dll "C:\path\to\libmpv-2.dll"
param([Parameter(Mandatory)][string]$Dll)
$ErrorActionPreference = 'Stop'

$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = & $vswhere -latest -products * -property installationPath
$bin = Get-ChildItem "$vs\VC\Tools\MSVC\*\bin\Hostx64\x64" | Sort-Object Name -Descending | Select-Object -First 1
if (-not $bin) { throw "MSVC x64 tools not found under $vs" }

$out = Join-Path $PSScriptRoot '..\vendor\mpv-dev'
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item $Dll (Join-Path $out 'libmpv-2.dll') -Force

$exports = & "$($bin.FullName)\dumpbin.exe" /exports $Dll
$names = $exports | Select-String '^\s+\d+\s+[0-9A-F]+\s+[0-9A-F]{8}\s+(mpv_\w+)' | ForEach-Object { $_.Matches[0].Groups[1].Value }
if (-not $names) { throw "No mpv_* exports found in $Dll" }
$def = Join-Path $out 'mpv.def'
@('LIBRARY libmpv-2.dll', 'EXPORTS') + $names | Set-Content -Encoding ascii $def
$lib = Join-Path $out 'mpv.lib'
& "$($bin.FullName)\lib.exe" /nologo "/def:$def" /machine:x64 "/out:$lib" | Out-Null
if (-not (Test-Path $lib)) { throw "lib.exe did not produce $lib" }
# Only mpv.lib and the DLL are needed; drop lib.exe's intermediate files.
Remove-Item $def, (Join-Path $out 'mpv.exp') -ErrorAction SilentlyContinue
Write-Host "Staged $($names.Count) exports in $out"
