param(
    [string]$Component = '',
    [string]$EvidenceDirectory = '',
    [switch]$Launch,
    [switch]$Relaunch
)

# Explicit ordinary-user Debug composition. No UAC, WFP, loopback exemptions,
# personal certificates, in-process preview or release-mode promotion.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($Launch -and $Relaunch) { throw 'Select -Launch or -Relaunch.' }
$Repository = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if ($Relaunch) {
    & (Join-Path $PSScriptRoot 'desktop-browser-windows-launch.ps1') -Repository $Repository
    return
}
if ($PSVersionTable.PSVersion.Major -lt 7) {
    throw 'Build/acceptance requires PowerShell 7: pwsh -NoProfile -ExecutionPolicy Bypass -File .\scripts\desktop-browser-windows.ps1 -Launch. Use -Relaunch to reopen an already prepared app from Windows PowerShell.'
}
if (-not $IsWindows -or [Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') {
    throw 'Use PowerShell 7 on native Windows x64.'
}
$Principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if ($Principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Run this acceptance from an ordinary, non-elevated Windows shell.'
}
$Allocation = [Guid]::NewGuid().ToString('N')
$Evidence = if ($EvidenceDirectory) { [IO.Path]::GetFullPath($EvidenceDirectory, $Repository) } else {
    Join-Path $Repository ".local/windows-browser-development-$Allocation"
}
if (Test-Path -LiteralPath $Evidence) { throw 'Preserve previous evidence; select a fresh directory.' }
$null = New-Item -ItemType Directory -Path $Evidence
$Development = Join-Path ([Environment]::GetFolderPath('UserProfile')) "ColossusBrowserDev-$Allocation"
$SavedEnvironment = @{}
foreach ($Name in @('PATH', 'COLOSSUS_HOME', 'LOCALAPPDATA', 'TEMP', 'TMP',
    'COLOSSUS_SIDECAR_WINDOWS_BROWSER_DEVELOPMENT', 'COLOSSUS_CEF_ROOT', 'COLOSSUS_CEF_NATIVE_LIB_DIR',
    'COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_COMPONENT', 'COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_STATE_PARENT',
    'COLOSSUS_BROWSER_ACCEPTANCE_OWNED_TABS', 'COLOSSUS_BROWSER_ACCEPTANCE_NATIVE_INPUT',
    'COLOSSUS_BROWSER_ACCEPTANCE_VISUAL_HOLD_MS', 'COLOSSUS_BROWSER_ACCEPTANCE_CLOSE_AFTER_COMPLETION', 'TAURI_CONFIG')) {
    $SavedEnvironment[$Name] = [Environment]::GetEnvironmentVariable($Name, 'Process')
}
function Invoke-Logged([string]$Command, [string[]]$Arguments, [string]$Log) {
    & $Command @Arguments *> (Join-Path $Evidence $Log)
    $Status = $LASTEXITCODE
    Get-Content -LiteralPath (Join-Path $Evidence $Log) -Tail 24
    if ($Status -ne 0) { throw "Native Windows development failed: $Command ($Log)" }
}
$Passed = $false
Push-Location $Repository
try {
    foreach ($Command in @('python', 'cmake', 'cargo', 'rustc', 'node')) {
        $null = Get-Command $Command -ErrorAction Stop
    }
    if (-not $Component) {
        & (Join-Path $PSScriptRoot 'browser-native-host-windows.ps1') *> (Join-Path $Evidence 'stage.log')
        if ($LASTEXITCODE -ne 0) { throw 'Private native browser staging failed; see stage.log.' }
        $Component = $env:COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_COMPONENT
    }
    $Component = [IO.Path]::GetFullPath($Component)
    Invoke-Logged python @('-B', 'native/browser/scripts/component.py', 'verify', '--root', $Component) 'inventory.log'
    $Selection = & python -B native/browser/scripts/windows_development.py --component $Component --destination $Development
    if ($LASTEXITCODE -ne 0 -or -not $Selection) { throw 'Private compiler selection allocation failed.' }
    $env:COLOSSUS_SIDECAR_WINDOWS_BROWSER_DEVELOPMENT = $Selection.Trim()
    $env:COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_COMPONENT = $Component
    $env:COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_STATE_PARENT = $Development
    $env:LOCALAPPDATA = $Development
    $env:TEMP = $Development
    $env:TMP = $Development
    $env:COLOSSUS_HOME = Join-Path $Development 'desktop-home'
    # Build the CLI/managed bundle through the repository composition owner,
    # then replace the Debug sidecar with the explicitly selected browser build.
    Invoke-Logged cargo @('xtask', 'desktop', 'prepare', '--profile', 'debug') 'prepare.log'
    Invoke-Logged cargo @('build', '--locked', '-p', 'colossus-sidecar', '--features', 'windows-browser-development') 'sidecar.log'
    $Binaries = Join-Path $Repository 'apps/desktop/src-tauri/binaries'
    Copy-Item -LiteralPath 'target/debug/colossus-sidecar.exe' -Destination (Join-Path $Binaries 'colossus-sidecar-x86_64-pc-windows-msvc.exe')
    $Cef = & python -B native/browser/scripts/component.py fetch --platform windows64
    if ($LASTEXITCODE -ne 0 -or -not $Cef) { throw 'Pinned CEF source unavailable.' }
    $env:COLOSSUS_CEF_ROOT = $Cef.Trim().Replace('\', '/')
    $env:COLOSSUS_CEF_NATIVE_LIB_DIR = Join-Path $Repository '.local/cef-build-windows64/Release'
    Invoke-Logged cargo @('test', '--locked', '-p', 'colossus-sandbox', '--test', 'browser_windows_native',
        'user_windows_chromium_automation_frames_input_transfers_and_cleanup', '--', '--ignored', '--exact', '--nocapture') 'native.log'
    Invoke-Logged cargo @('test', '--locked', '-p', 'colossus-runtime',
        'browser_tools::tests::native_windows::real_windows_chromium_executes_core_tools_and_paints_authenticated_frames',
        '--', '--ignored', '--exact', '--nocapture') 'core.log'
    Invoke-Logged cargo @('build', '--locked', '--manifest-path', 'apps/desktop/src-tauri/Cargo.toml',
        '--no-default-features', '--features', 'windows-browser-development', '--example', 'browser-acceptance') 'desktop-build.log'
    $env:PATH = $Component + ';' + $env:PATH
    $env:COLOSSUS_BROWSER_ACCEPTANCE_OWNED_TABS = '1'
    $env:COLOSSUS_BROWSER_ACCEPTANCE_NATIVE_INPUT = $null
    $env:COLOSSUS_BROWSER_ACCEPTANCE_VISUAL_HOLD_MS = '0'
    $env:COLOSSUS_BROWSER_ACCEPTANCE_CLOSE_AFTER_COMPLETION = $null
    Invoke-Logged 'apps/desktop/src-tauri/target/debug/examples/browser-acceptance.exe' @('http://127.0.0.1:53713/') 'desktop.log'
    foreach ($Name in @('windows-human-foreground.png', 'windows-overlay-foreground.png', 'windows-agent-foreground.png')) {
        Copy-Item -LiteralPath (Join-Path $env:COLOSSUS_HOME $Name) -Destination (Join-Path $Evidence $Name)
    }
    $env:COLOSSUS_BROWSER_ACCEPTANCE_OWNED_TABS = $null
    $env:COLOSSUS_BROWSER_ACCEPTANCE_CLOSE_AFTER_COMPLETION = '1'
    $env:COLOSSUS_HOME = Join-Path $Development 'desktop-after-completion-home'
    Invoke-Logged 'apps/desktop/src-tauri/target/debug/examples/browser-acceptance.exe' @('http://127.0.0.1:53713/') 'desktop-after-completion.log'
    Invoke-Logged python @('-B', 'native/browser/scripts/component.py', 'verify', '--root', $Component) 'inventory-after.log'
    if (@(Get-ChildItem -LiteralPath (Join-Path $Development 'state') -Force).Count -ne 0) {
        throw 'Supervised native state survived acknowledged Desktop shutdown.'
    }
    $Passed = $true
    if ($Launch) {
        # The installed app may already own its single-instance identifier.
        # Keep this source test window independently launchable.
        $env:TAURI_CONFIG = '{"identifier":"com.obscuritylabs.colossus.desktop.windows-browser-development","productName":"Colossus Browser Test"}'
        Invoke-Logged cargo @('build', '--locked', '--manifest-path', 'apps/desktop/src-tauri/Cargo.toml',
            '--no-default-features', '--features', 'windows-browser-development', '--bin', 'colossus-desktop') 'application-build.log'
        $Application = Join-Path $Repository 'apps/desktop/src-tauri/target/debug/colossus-desktop.exe'
        [ordered]@{
            schema = 1
            application_sha256 = (Get-FileHash -LiteralPath $Application -Algorithm SHA256).Hash.ToLowerInvariant()
            component = $Component
            development_root = $Development
            home = Join-Path $Development 'application-home'
        } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Repository '.local/windows-browser-launch.json') -Encoding UTF8
        & (Join-Path $PSScriptRoot 'desktop-browser-windows-launch.ps1') -Repository $Repository
    }
} finally {
    [IO.File]::WriteAllText((Join-Path $Evidence 'acceptance.json'), (([ordered]@{
        schema = 1; passed = $Passed; administrator = $false; developmentOnly = $true
        component = $Component; developmentRoot = $Development
        chromiumSandbox = $true; outerContainmentAccepted = $false
        privateCertificatesAccepted = $false; installedSignedPackageAccepted = $false
        productionDesktop = $false; productionHeadless = $false
    } | ConvertTo-Json) + "`n"), [Text.UTF8Encoding]::new($false))
    foreach ($Name in $SavedEnvironment.Keys) {
        [Environment]::SetEnvironmentVariable($Name, $SavedEnvironment[$Name], 'Process')
    }
    Pop-Location
}
Write-Host "Non-admin Windows browser automation evidence: $Evidence"
