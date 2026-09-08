param(
    [Parameter(Mandatory = $true)]
    [string]$Binary
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true
. (Join-Path $PSScriptRoot "windows-release-fixture.ps1")
$binaryPath = (Get-Item -LiteralPath $Binary).FullName
$fixture = New-ColossusReleaseFixture
$savedEnvironment = @{}
foreach ($name in @("COLOSSUS_HOME", "COLOSSUS_RELEASE_JOURNAL_KEY", "COLOSSUS_RELEASE_SIGNING_KEY")) {
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
}
try {
    $env:COLOSSUS_HOME = Join-Path $fixture "colossus-home"
    $env:COLOSSUS_RELEASE_JOURNAL_KEY = "5555555555555555555555555555555555555555555555555555555555555555"
    $env:COLOSSUS_RELEASE_SIGNING_KEY = "6666666666666666666666666666666666666666666666666666666666666666"
    Copy-Item (Join-Path $PSScriptRoot "../../release/smoke-config.yaml") (Join-Path $fixture "config.yaml")
    New-Item -ItemType Directory -Path (Join-Path $fixture "workflows") | Out-Null
    Push-Location $fixture
    try {
        # Exercise the same runtime bootstrap that failed in the release job.
        & $binaryPath --config config.yaml config show | Out-Null
        $plugins = @(& $binaryPath --config config.yaml plugins list | ConvertFrom-Json)
        if ($plugins.Count -ne 1 -or $plugins[0].manifest.name -ne "colossus" -or
            $plugins[0].origin -ne "bundled" -or -not $plugins[0].available -or
            $plugins[0].skills.Count -ne 4) {
            throw "private release fixture did not bootstrap the embedded core"
        }
    } finally {
        Pop-Location
    }
    "Owner-private Windows release fixture passed runtime bootstrap."
} finally {
    foreach ($name in $savedEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], "Process")
    }
    Remove-Item -LiteralPath $fixture -Recurse -Force
}
