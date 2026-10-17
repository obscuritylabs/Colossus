param([Parameter(Mandatory = $true)][string]$Repository)

# Launch only the exact prepared source app. Compatible with Windows PowerShell
# 5.1; no compiler, Python, CMake, elevation or machine policy changes are needed.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
    throw 'The Windows browser development app requires Windows.'
}
$Principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if ($Principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Launch the app from an ordinary, non-elevated shell.'
}
$Record = Join-Path $Repository '.local/windows-browser-launch.json'
if (-not (Test-Path -LiteralPath $Record -PathType Leaf)) {
    throw 'No prepared browser app exists. Run desktop-browser-windows.ps1 -Launch with PowerShell 7 first.'
}
$Prepared = Get-Content -LiteralPath $Record -Raw | ConvertFrom-Json
$Application = Join-Path $Repository 'apps/desktop/src-tauri/target/debug/colossus-desktop.exe'
if ($Prepared.schema -ne 1 -or -not (Test-Path -LiteralPath $Application -PathType Leaf) -or
    (Get-FileHash -LiteralPath $Application -Algorithm SHA256).Hash.ToLowerInvariant() -ne $Prepared.application_sha256) {
    throw 'The prepared browser app changed. Rebuild and accept it with -Launch before reopening.'
}
foreach ($Path in @($Prepared.component, $Prepared.development_root, $Prepared.home)) {
    if (-not [IO.Path]::IsPathRooted($Path) -or $Path.Contains(';')) {
        throw 'Prepared browser paths must be absolute and cannot contain PATH separators.'
    }
}
if (-not (Test-Path -LiteralPath (Join-Path $Prepared.component 'browser-component.json') -PathType Leaf) -or
    -not (Test-Path -LiteralPath $Prepared.development_root -PathType Container)) {
    throw 'The prepared native browser component or state allocation is missing.'
}
$ExpectedHome = [IO.Path]::GetFullPath((Join-Path $Prepared.development_root 'application-home'))
if ([IO.Path]::GetFullPath($Prepared.home) -ne $ExpectedHome) {
    throw 'The app profile differs from its prepared development allocation.'
}
$Saved = @{}
foreach ($Name in @('PATH', 'COLOSSUS_HOME', 'LOCALAPPDATA', 'TEMP', 'TMP')) {
    $Saved[$Name] = [Environment]::GetEnvironmentVariable($Name, 'Process')
}
try {
    $env:COLOSSUS_HOME = $Prepared.home
    $env:LOCALAPPDATA = $Prepared.development_root
    $env:TEMP = $Prepared.development_root
    $env:TMP = $Prepared.development_root
    $env:PATH = $Prepared.component + ';' + $env:PATH
    $Log = Join-Path $Repository ('.local/windows-browser-launch-' + [Guid]::NewGuid().ToString('N'))
    $App = Start-Process -FilePath $Application -WorkingDirectory (Split-Path -Parent $Application) `
        -WindowStyle Normal -RedirectStandardOutput ($Log + '.stdout.log') `
        -RedirectStandardError ($Log + '.stderr.log') -PassThru
    Write-Host "Opened browser development app (PID $($App.Id))."
    Write-Host "Test profile: $($Prepared.home)"
} finally {
    foreach ($Name in $Saved.Keys) {
        [Environment]::SetEnvironmentVariable($Name, $Saved[$Name], 'Process')
    }
}
