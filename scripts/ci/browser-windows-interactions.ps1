param(
    [Parameter(Mandatory = $true)][string]$Component,
    [string]$EvidenceDirectory = '.local/windows-browser-interactions'
)
# Ordinary-user source acceptance. Production modes and certificate/egress
# acceptance remain false. No UAC, personal profile or debugging endpoint.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $IsWindows -or [Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') {
    throw 'Use PowerShell 7 on Windows x64.'
}
$Principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if ($Principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'This acceptance requires an ordinary-user Windows token.'
}
$Repository = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$Evidence = if ([IO.Path]::IsPathRooted($EvidenceDirectory)) { $EvidenceDirectory } else { Join-Path $Repository $EvidenceDirectory }
if (Test-Path -LiteralPath $Evidence) { throw 'Use a fresh evidence directory for each attempt.' }
$null = New-Item -ItemType Directory -Path $Evidence
$Saved = @{}
$Passed = $false
$Parent = $null
$TestExit = $null
Push-Location $Repository
try {
    & python -B native/browser/scripts/component.py verify --root $Component
    if ($LASTEXITCODE -ne 0) { throw 'Component verification failed.' }
    $ManifestPath = Join-Path $Component 'browser-component.json'
    $Manifest = Get-Content -LiteralPath $ManifestPath -Raw | ConvertFrom-Json
    if ($Manifest.modes.desktop -ne $false -or $Manifest.modes.headless -ne $false) {
        throw 'Both production modes must remain disabled.'
    }
    $Parent = Join-Path ([Environment]::GetFolderPath('UserProfile')) ('ColossusBrowserInteractions-' + [Guid]::NewGuid().ToString('N'))
    & python -B -c 'import sys; from pathlib import Path; sys.path.insert(0, "native/browser/scripts"); from pki_fixture_private import private_directory; private_directory(Path(sys.argv[1]))' $Parent
    if ($LASTEXITCODE -ne 0) { throw 'Private acceptance allocation failed.' }
    foreach ($Name in @('LOCALAPPDATA','TEMP','TMP','COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_COMPONENT','COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_STATE_PARENT','COLOSSUS_BROWSER_WINDOWS_ANNOTATION_EVIDENCE')) {
        $Saved[$Name] = [Environment]::GetEnvironmentVariable($Name,'Process')
    }
    foreach ($Name in @('LOCALAPPDATA','TEMP','TMP')) { [Environment]::SetEnvironmentVariable($Name,$Parent,'Process') }
    $env:COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_COMPONENT = $Component
    $env:COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_STATE_PARENT = $Parent
    $env:COLOSSUS_BROWSER_WINDOWS_ANNOTATION_EVIDENCE = Join-Path $Evidence 'annotations.png'
    $Lines = [Collections.Generic.Queue[string]]::new()
    $Bytes = 0
    $Proof = $false
    & cargo test --locked -p colossus-runtime --lib browser_tools::tests::native_windows::real_windows_chromium_executes_core_tools_and_paints_authenticated_frames -- --ignored --exact --nocapture 2>&1 | ForEach-Object {
        $Line = [string]$_
        Write-Host $Line
        if ($Line.StartsWith('PASS real Windows Core hover,')) { $Proof = $true }
        if ($Line.Length -gt 16384) { $Line = $Line.Substring(0,16384) }
        $Lines.Enqueue($Line)
        $Bytes += [Text.Encoding]::UTF8.GetByteCount($Line) + 1
        while ($Bytes -gt 256KB -and $Lines.Count -gt 0) { $Bytes -= [Text.Encoding]::UTF8.GetByteCount($Lines.Dequeue()) + 1 }
    }
    $TestExit = $LASTEXITCODE
    [IO.File]::WriteAllText((Join-Path $Evidence 'native.log'),($Lines -join "`n") + "`n",[Text.UTF8Encoding]::new($false))
    if ($TestExit -ne 0 -or -not $Proof -or -not (Test-Path -LiteralPath $env:COLOSSUS_BROWSER_WINDOWS_ANNOTATION_EVIDENCE)) {
        throw 'Native interaction acceptance failed; preserve the log and private allocation for diagnosis.'
    }
    # Runtime deliberately retains zero-byte worker lock tombstones. This
    # allocation is unique to the now-exited test process; retire only that
    # exact known shape, never arbitrary retained browser or application data.
    $LeaseDirectory = Join-Path $Parent 'colossus-worker-leases'
    if (Test-Path -LiteralPath $LeaseDirectory) {
        $ResolvedParent = [IO.Path]::GetFullPath($Parent).TrimEnd('\') + '\'
        $ResolvedLeases = [IO.Path]::GetFullPath($LeaseDirectory)
        if (-not $ResolvedLeases.StartsWith($ResolvedParent,[StringComparison]::OrdinalIgnoreCase) -or
            ((Get-Item -LiteralPath $LeaseDirectory).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'Worker tombstone ancestry changed.'
        }
        $Locks = @(Get-ChildItem -LiteralPath $LeaseDirectory -Force)
        foreach ($Lock in $Locks) {
            if ($Lock.PSIsContainer -or $Lock.Name -notmatch '^[0-9a-f]{64}\.lock$' -or $Lock.Length -ne 0 -or
                ($Lock.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Unexpected retained worker data.' }
        }
        foreach ($Lock in $Locks) { Remove-Item -LiteralPath $Lock.FullName }
        Remove-Item -LiteralPath $LeaseDirectory
    }
    if ((Get-ChildItem -LiteralPath $Parent -Force | Measure-Object).Count -ne 0) {
        throw 'Native acceptance retained private state after shutdown.'
    }
    $Passed = $true
} finally {
    foreach ($Name in $Saved.Keys) { [Environment]::SetEnvironmentVariable($Name,$Saved[$Name],'Process') }
    $Receipt = [ordered]@{
        schema_version=1; platform='windows'; standard_user=$true; component=$Component;
        passed=$Passed; test_exit=$TestExit; private_state_parent=$Parent;
        production_availability=$false; outer_containment_accepted=$false;
        certificate_custody_accepted=$false; installed_signed_package_accepted=$false
    }
    [IO.File]::WriteAllText((Join-Path $Evidence 'acceptance.json'),($Receipt | ConvertTo-Json) + "`n",[Text.UTF8Encoding]::new($false))
    Pop-Location
}
