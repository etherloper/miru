# Builds and launches the Miru dev build. The first time, if no TMDB key is configured, it
# asks for one at a hidden prompt and saves it to .env in the project folder (gitignored), so
# later runs - including plain `cargo run -p miru` - pick it up automatically.
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..')
$envFile = Join-Path $root '.env'

$configured = [bool]$env:TMDB_API_KEY -or (
    (Test-Path $envFile) -and
    (Select-String -Path $envFile -Pattern '^\s*(export\s+)?TMDB_API_KEY\s*=\s*\S' -Quiet)
)
if (-not $configured) {
    $secure = Read-Host 'Paste your TMDB API key or read access token (saved to .env)' -AsSecureString
    $key = [System.Net.NetworkCredential]::new('', $secure).Password.Trim()
    if (-not $key) { throw 'No TMDB key entered.' }
    # .NET writes UTF-8 without a byte-order mark; start on a new line if the file has content.
    $prefix = if ((Test-Path $envFile) -and (Get-Item $envFile).Length -gt 0) { "`n" } else { '' }
    [System.IO.File]::AppendAllText($envFile, "${prefix}TMDB_API_KEY=$key`n")
    Write-Host "Saved to $envFile"
}

Push-Location $root
try {
    # Always run the latest code.
    cargo build -p miru
    if ($LASTEXITCODE -ne 0) { throw 'Build failed.' }
    & '.\target\debug\miru.exe'
} finally {
    Pop-Location
}
