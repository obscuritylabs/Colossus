$ErrorActionPreference = "Stop"
$Repository = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Desktop = Join-Path $Repository "apps/desktop"
$Connection = Join-Path $Desktop "src-tauri/connection.local.json"

if (Test-Path $Connection) {
    $Value = Get-Content -Raw $Connection | ConvertFrom-Json
    $Instance = [string]$Value.instanceId -replace "-", ""
    $Fingerprint = [string]$Value.certificateSha256
    if ($Instance -notmatch "^[0-9a-fA-F]{32}$" -or
        $Instance -match "^0+$" -or
        $Fingerprint -notmatch "^[0-9a-fA-F]{64}$" -or
        $Fingerprint -match "^0+$") {
        throw "connection.local.json has invalid External target trust values"
    }
}

Push-Location $Desktop
try {
    npm ci --ignore-scripts
    if ($LASTEXITCODE -ne 0) {
        throw "desktop dependency installation failed"
    }
    npm run tauri:dev
    if ($LASTEXITCODE -ne 0) {
        throw "desktop development application failed"
    }
}
finally {
    Pop-Location
}
