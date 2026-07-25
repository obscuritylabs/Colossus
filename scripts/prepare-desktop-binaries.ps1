param(
    [Parameter(Mandatory = $true)]
    [ValidateSet("debug", "release")]
    [string]$Profile
)

$ErrorActionPreference = "Stop"
$Repository = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Push-Location $Repository
try {
    cargo xtask desktop prepare --profile $Profile
    if ($LASTEXITCODE -ne 0) {
        throw "desktop binary preparation failed"
    }
}
finally {
    Pop-Location
}
