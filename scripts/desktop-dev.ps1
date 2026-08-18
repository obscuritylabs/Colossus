$ErrorActionPreference = "Stop"
$Repository = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Desktop = Join-Path $Repository "apps/desktop"
$Connection = Join-Path $Desktop "src-tauri/connection.local.json"

function Test-AbsoluteDevPath {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Path
    )

    if ([string]::IsNullOrWhiteSpace($Path)) {
        return $false
    }

    $Root = [System.IO.Path]::GetPathRoot($Path)
    if ([string]::IsNullOrWhiteSpace($Root)) {
        return $false
    }
    if ($Root -eq "\") {
        return $false
    }
    if ($Root -match "^[A-Za-z]:$") {
        return $false
    }
    return $true
}

function Assert-AbsoluteDevPath {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Name,
        [Parameter(Mandatory = $true)]
        [string] $Path
    )

    if (-not (Test-AbsoluteDevPath -Path $Path)) {
        throw (
            "{0} must be an absolute private directory for desktop development: {1}" -f
                $Name, $Path
        )
    }
}

function Set-PrivateDevPathAcl {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Path
    )

    if (-not (Test-Path -LiteralPath $Path)) {
        return
    }

    $Item = Get-Item -LiteralPath $Path -Force
    $CurrentUserSid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    if ($Item.PSIsContainer) {
        & icacls $Path /inheritance:r /grant:r `
            "*$($CurrentUserSid):(OI)(CI)F" `
            "*S-1-5-18:(OI)(CI)F" `
            "*S-1-5-32-544:(OI)(CI)F" | Out-Null
    }
    else {
        & icacls $Path /inheritance:r /grant:r `
            "*$($CurrentUserSid):F" `
            "*S-1-5-18:F" `
            "*S-1-5-32-544:F" | Out-Null
    }
    if ($LASTEXITCODE -ne 0) {
        throw (
            "failed to make the dev credential path private: {0}" -f $Path
        )
    }
}

function Ensure-PrivateDevDirectory {
    param(
        [Parameter(Mandatory = $true)]
        [string] $Path
    )

    New-Item -ItemType Directory -Force -Path $Path | Out-Null
    Set-PrivateDevPathAcl -Path $Path
    Get-ChildItem -LiteralPath $Path -Force -Recurse -ErrorAction SilentlyContinue |
        ForEach-Object { Set-PrivateDevPathAcl -Path $_.FullName }
}

if ([string]::IsNullOrWhiteSpace($env:COLOSSUS_HOME)) {
    $LocalAppData = [Environment]::GetFolderPath("LocalApplicationData")
    if (-not $LocalAppData) {
        throw "LOCALAPPDATA is unavailable; set COLOSSUS_HOME to an absolute private directory before starting desktop development"
    }
    $env:COLOSSUS_HOME = Join-Path $LocalAppData "ColossusDevHome"
    Write-Host "Using COLOSSUS_HOME=$env:COLOSSUS_HOME for desktop development"
}
Assert-AbsoluteDevPath -Name "COLOSSUS_HOME" -Path $env:COLOSSUS_HOME
Ensure-PrivateDevDirectory -Path $env:COLOSSUS_HOME

if ([string]::IsNullOrWhiteSpace($env:CODEX_HOME)) {
    $LocalAppData = [Environment]::GetFolderPath("LocalApplicationData")
    if (-not $LocalAppData) {
        throw "LOCALAPPDATA is unavailable; set CODEX_HOME to an absolute private directory before using Codex login"
    }
    $env:CODEX_HOME = Join-Path $LocalAppData "ColossusDevCodexHome"
    Write-Host "Using CODEX_HOME=$env:CODEX_HOME for desktop Codex login"
}
Assert-AbsoluteDevPath -Name "CODEX_HOME" -Path $env:CODEX_HOME
Ensure-PrivateDevDirectory -Path $env:CODEX_HOME

if ([string]::IsNullOrWhiteSpace($env:COLOSSUS_CODEX_BIN)) {
    $CodexCandidates = @()
    $CodexBinRoot = Join-Path $env:LOCALAPPDATA "OpenAI\Codex\bin"
    if (Test-Path $CodexBinRoot) {
        $CodexCandidates += Get-ChildItem -LiteralPath $CodexBinRoot -Filter "codex.exe" -Recurse -File |
            Sort-Object LastWriteTime -Descending |
            Select-Object -ExpandProperty FullName
    }
    $CodexCommand = Get-Command codex.exe -ErrorAction SilentlyContinue
    if ($CodexCommand -and $CodexCommand.Source) {
        $CodexCandidates += $CodexCommand.Source
    }
    foreach ($Candidate in ($CodexCandidates | Select-Object -Unique)) {
        if (Test-Path -LiteralPath $Candidate) {
            $env:COLOSSUS_CODEX_BIN = $Candidate
            Write-Host "Using COLOSSUS_CODEX_BIN=$env:COLOSSUS_CODEX_BIN for ChatGPT sign-in"
            break
        }
    }
    if ([string]::IsNullOrWhiteSpace($env:COLOSSUS_CODEX_BIN)) {
        Write-Warning "Codex CLI was not found. Install Codex or set COLOSSUS_CODEX_BIN to an absolute codex.exe before using ChatGPT sign-in."
    }
}

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

Push-Location $Repository
try {
    cargo xtask desktop prepare --profile debug
    if ($LASTEXITCODE -ne 0) {
        throw "desktop managed runtime preparation failed"
    }
}
finally {
    Pop-Location
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
