param(
    [string]$EvidenceDirectory = '.local/windows-owned-browser-evidence',
    [string]$Component = '',
    [ValidateSet('Containment', 'Development')][string]$Mode = 'Containment'
)

# Reviewed premerge diagnostic only. This script never accepts a release mode or
# copies profiles, transferred bytes, bootstrap keys or private channels as evidence.
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
if (-not $IsWindows -or [Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne "X64") {
    throw "Native browser acceptance requires Windows x64 and PowerShell 7."
}
$Repository = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
$Evidence = if ([IO.Path]::IsPathRooted($EvidenceDirectory)) { $EvidenceDirectory } else { Join-Path $Repository $EvidenceDirectory }
if (Test-Path -LiteralPath $Evidence) { throw "Native evidence destination must be fresh." }
$null = New-Item -ItemType Directory -Path $Evidence
$Parent = $null
$Passed = $false
$ManifestHash = $null
$Development = $Mode -eq 'Development'
$Principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if ($Development -and $Principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'Standard-user development acceptance must run without an elevated Windows token.'
}
$Checks = @(
    "native_windows_factory",
    "authenticated_bgra", "native_input", "read_only_viewer_detach",
    "exact_origin_denial", "native_upload_actual_http_bytes", "native_download_actual_http_bytes",
    "full_job_and_io_cleanup", "cef_shutdown", "native_typed_automation", "native_tabs"
)
if (-not $Development) { $Checks += @('low_appcontainer_token', 'low_package_profile_label') }
$Checked = @{}
foreach ($Check in $Checks) { $Checked[$Check] = $false }
Push-Location $Repository
try {
    if ([string]::IsNullOrWhiteSpace($Component)) {
        & (Join-Path $Repository "scripts/browser-native-host-windows.ps1")
        $Component = $env:COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_COMPONENT
    } else {
        & python -B native/browser/scripts/component.py verify --root $Component
        if ($LASTEXITCODE -ne 0) { throw "Selected diagnostic component verification failed." }
    }
    if (-not $Component) { throw "The builder did not select its fresh verified component." }
    $Manifest = Join-Path $Component "browser-component.json"
    if ((Get-Item -LiteralPath $Manifest).Length -gt 8MB) { throw "Component inventory exceeded its evidence bound." }
    $Document = Get-Content -LiteralPath $Manifest -Raw | ConvertFrom-Json
    if ($Document.modes.desktop -ne $false -or $Document.modes.headless -ne $false) {
        throw "Source diagnostic requires both release modes to remain false."
    }
    $ManifestHash = (Get-FileHash -LiteralPath $Manifest -Algorithm SHA256).Hash.ToLowerInvariant()
    Copy-Item -LiteralPath $Manifest -Destination (Join-Path $Evidence "browser-component.json")
    $env:COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_COMPONENT = $Component
    $UserProfile = [Environment]::GetFolderPath('UserProfile')
    if ([string]::IsNullOrWhiteSpace($UserProfile)) { throw "Native acceptance requires an owned user profile." }
    $Parent = Join-Path $UserProfile ("ColossusBrowserCI-" + [Guid]::NewGuid().ToString("N"))
    & python -B -c 'import sys; from pathlib import Path; sys.path.insert(0, "native/browser/scripts"); from pki_fixture_private import private_directory; private_directory(Path(sys.argv[1]))' $Parent
    if ($LASTEXITCODE -ne 0) { throw "Private fixture state allocation failed." }
    $env:COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_STATE_PARENT = $Parent
    $SavedPaths = @{}
    foreach ($Name in @('LOCALAPPDATA', 'TEMP', 'TMP')) {
        $SavedPaths[$Name] = [Environment]::GetEnvironmentVariable($Name, 'Process')
        [Environment]::SetEnvironmentVariable($Name, $Parent, 'Process')
    }
    # A rolling categorical log stays bounded even on a noisy compiler failure.
    $Lines = [Collections.Generic.Queue[string]]::new()
    $Bytes = 0
    $Receipt = $null
    $Fixture = if ($Development) { 'user_windows_chromium_automation_frames_input_transfers_and_cleanup' } else {
        'owned_windows_chromium_frames_input_viewer_and_full_cleanup'
    }
    & cargo test --locked -p colossus-sandbox --test browser_windows_native $Fixture `
        -- --ignored --exact --nocapture 2>&1 | ForEach-Object {
        $Line = [string]$_
        Write-Host $Line
        if ($Line.Length -gt 16384) { $Line = $Line.Substring(0, 16384) }
        $Lines.Enqueue($Line)
        $Bytes += [Text.Encoding]::UTF8.GetByteCount($Line) + 1
        while ($Bytes -gt 256KB -and $Lines.Count -gt 0) {
            $Bytes -= [Text.Encoding]::UTF8.GetByteCount($Lines.Dequeue()) + 1
        }
        if ($Line.StartsWith('{') -and $Line.Contains('"native_windows_factory"')) {
            try {
                $Candidate = $Line | ConvertFrom-Json
                if ($Candidate.native_windows_factory -eq $true) { $Receipt = $Candidate }
            } catch { $Receipt = $null }
        }
    }
    $TestStatus = $LASTEXITCODE
    [IO.File]::WriteAllText((Join-Path $Evidence "native-factory.log"), ($Lines -join "`n") + "`n", [Text.UTF8Encoding]::new($false))
    if ($TestStatus -ne 0 -or $null -eq $Receipt) { throw "The real native fixture failed or produced no exact acceptance receipt." }
    foreach ($Check in $Checks) {
        $Value = $Receipt.PSObject.Properties[$Check]
        if ($null -eq $Value -or $Value.Value -isnot [bool] -or $Value.Value -ne $true) {
            throw "Native fixture did not prove mandatory evidence: $Check"
        }
        $Checked[$Check] = $true
    }
    foreach ($Check in @("whole_host_network_acceptance", "production_containment")) {
        $Value = $Receipt.PSObject.Properties[$Check]
        if ($null -eq $Value -or $Value.Value -isnot [bool] -or $Value.Value -ne $false) {
            throw "A source diagnostic cannot accept production containment."
        }
    }
    if ($Receipt.standard_user_development -ne $Development -or
        $Receipt.low_appcontainer_token -ne (-not $Development) -or
        $Receipt.low_package_profile_label -ne (-not $Development)) {
        throw 'Native receipt described a different execution mode.'
    }
    # The actual fixture removes its owned subtree only after the complete native
    # shutdown receipt. Nonrecursive removal refuses any unknown surviving state.
    [IO.Directory]::Delete($Parent)
    $Parent = $null
    $Passed = $true
} finally {
    $Report = [ordered]@{
        schema = 1
        passed = $Passed
        sourceDiagnostic = $true
        standardUserDevelopment = $Development
        componentManifestSha256 = $ManifestHash
        checks = $Checked
        privateStatePreserved = $null -ne $Parent
        acceptedDesktop = $false
        acceptedHeadless = $false
        productionContainment = $false
    }
    [IO.File]::WriteAllText((Join-Path $Evidence "native-factory.json"), ($Report | ConvertTo-Json -Depth 4) + "`n", [Text.UTF8Encoding]::new($false))
    if (Get-Variable -Name SavedPaths -ErrorAction SilentlyContinue) {
        foreach ($Name in $SavedPaths.Keys) { [Environment]::SetEnvironmentVariable($Name, $SavedPaths[$Name], 'Process') }
    }
    Pop-Location
}
