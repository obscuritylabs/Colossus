param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('build', 'seal')]
    [string]$Phase,
    [string]$AllocationRoot = $env:COLOSSUS_BROWSER_PREVIEW_ALLOCATION,
    [switch]$RequireSignatures
)

# Publisher build only. No browser process is launched and no accepted production
# mode is promoted. Signing must finish before a fresh inventory is sealed.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $IsWindows -or [Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') {
    throw 'Windows functional-preview staging requires PowerShell7 on Windowsx64.'
}
if ($env:COLOSSUS_DESKTOP_RELEASE_CHANNEL -notin @('developer_preview', 'validation_only')) {
    throw 'The functional browser belongs only to the explicit preview edition.'
}
if ($env:COLOSSUS_DESKTOP_RELEASE_CHANNEL -eq 'developer_preview' -and $Phase -eq 'seal' -and -not $RequireSignatures) {
    throw 'A published preview requires signatures before inventory sealing.'
}
$Repository = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not $AllocationRoot) {
    if ($Phase -ne 'build') { throw 'The original build allocation is required.' }
    $AllocationRoot = Join-Path $Repository "apps/desktop/src-tauri/binaries/browser-preview-$([Guid]::NewGuid().ToString('N'))"
}
$AllocationRoot = [IO.Path]::GetFullPath($AllocationRoot)
$Component = Join-Path $AllocationRoot 'component'
$Seal = Join-Path $AllocationRoot 'windows-browser-functional-preview.json'
function Invoke-Checked([string]$Command, [string[]]$Arguments) {
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) { throw "Functional-preview staging failed: $Command" }
}
Push-Location $Repository
try {
    if ($Phase -eq 'build') {
        if (Test-Path -LiteralPath $AllocationRoot) { throw 'Select a fresh preview build allocation.' }
        $null = New-Item -ItemType Directory -Path $AllocationRoot
        $Cef = (& python -B native/browser/scripts/component.py fetch --platform windows64).Trim()
        if ($LASTEXITCODE -ne 0 -or -not $Cef) { throw 'Pinned CEF acquisition failed.' }
        $Cef = $Cef.Replace('\', '/')
        $Build = Join-Path $AllocationRoot 'native-build'
        Invoke-Checked cmake @('-S', 'native/browser', '-B', $Build, '-A', 'x64',
            "-DCEF_ROOT=$Cef", '-DCEF_RUNTIME_LIBRARY_FLAG=/MD', '-DUSE_SANDBOX=ON')
        Invoke-Checked cmake @('--build', $Build, '--config', 'Release', '--parallel', '4')
        $env:COLOSSUS_CEF_ROOT = $Cef
        $env:COLOSSUS_CEF_NATIVE_LIB_DIR = Join-Path $Build 'Release'
        $DriverTarget = Join-Path $AllocationRoot 'driver-build'
        Invoke-Checked cargo @('build', '--locked', '--release', '--lib', '--features',
            'windows-browser-functional-preview', '--manifest-path', 'native/browser/driver/Cargo.toml',
            '--target-dir', $DriverTarget)
        $Client = Join-Path $AllocationRoot 'client.dll'
        Copy-Item -LiteralPath (Join-Path $DriverTarget 'release/colossus_native_browser_host.dll') -Destination $Client
        try {
            Invoke-Checked python @('-B', 'native/browser/scripts/stage_windows.py', '--host', '--defer-inventory',
                '--cef-root', $Cef, '--native-build', $env:COLOSSUS_CEF_NATIVE_LIB_DIR,
                '--client-dll', $Client, '--destination', $Component)
        } finally { Remove-Item -LiteralPath $Client -ErrorAction SilentlyContinue }
        $env:COLOSSUS_BROWSER_PRESENTER_NATIVE_LIB_DIR = Join-Path $Build 'Release'
    } else {
        if (-not (Test-Path -LiteralPath $Component -PathType Container) -or
            (Test-Path -LiteralPath (Join-Path $Component 'browser-component.json')) -or
            (Test-Path -LiteralPath $Seal)) { throw 'Sealing requires the original fresh unsealed component.' }
        if ($RequireSignatures) {
            $Images = @(Get-ChildItem -LiteralPath $Component -Recurse -File |
                Where-Object Extension -In @('.exe', '.dll'))
            if ($Images.Count -lt 6 -or $Images.Count -gt 64) { throw 'Unexpected native browser image inventory.' }
            foreach ($Image in $Images) {
                & (Join-Path $PSScriptRoot 'ci/verify-authenticode.ps1') -Path $Image.FullName
            }
        }
        Invoke-Checked python @('-B', 'native/browser/scripts/component.py', 'inventory',
            '--root', $Component, '--platform', 'windows64', '--executable', 'colossus-native-browser-host.exe')
        Invoke-Checked python @('-B', 'native/browser/scripts/windows_preview.py', '--component', $Component, '--output', $Seal)
        $env:COLOSSUS_SIDECAR_WINDOWS_BROWSER_FUNCTIONAL_PREVIEW = $Seal
    }
    $env:COLOSSUS_BROWSER_PREVIEW_ALLOCATION = $AllocationRoot
    $env:COLOSSUS_BROWSER_PREVIEW_COMPONENT = $Component
    $env:COLOSSUS_DESKTOP_BROWSER_FUNCTIONAL_PREVIEW = '1'
    Write-Output $AllocationRoot
} finally { Pop-Location }
