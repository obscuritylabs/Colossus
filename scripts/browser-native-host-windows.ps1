# Build/stage only. The private host requires native supervisor-owned channels;
# it must not be launched directly or given renderer-nominated bootstrap paths.
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
if (-not $IsWindows -or [Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne "X64") {
    throw "The private Chromium host requires PowerShell 7 on Windows x64."
}
function Invoke-Checked([string]$Command, [string[]]$Arguments) {
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) { throw "Private Chromium host build failed: $Command" }
}
$Repository = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Push-Location $Repository
try {
    foreach ($Command in @("python", "cmake", "cargo", "rustc")) {
        $null = Get-Command $Command -ErrorAction Stop
    }
    $HostTriple = (& rustc --print host-tuple).Trim()
    if ($LASTEXITCODE -ne 0 -or $HostTriple -ne "x86_64-pc-windows-msvc") {
        throw "Use the native Windows x64 MSVC Rust toolchain."
    }
    $CefOutput = & python -B native/browser/scripts/component.py fetch --platform windows64
    if ($LASTEXITCODE -ne 0) { throw "Pinned CEF acquisition failed." }
    if ([string]::IsNullOrWhiteSpace($CefOutput)) { throw "Pinned CEF acquisition returned no source root." }
    # CEF's CMake macros expand the path as CMake source. Native backslashes
    # become escape sequences there; forward slashes preserve the literal path.
    $CefRoot = $CefOutput.Trim().Replace('\', '/')
    $Build = Join-Path $Repository ".local/cef-build-windows64"
    Invoke-Checked cmake @("-S", "native/browser", "-B", $Build, "-A", "x64", "-DCEF_ROOT=$CefRoot", "-DCEF_RUNTIME_LIBRARY_FLAG=/MD", "-DUSE_SANDBOX=ON")
    Invoke-Checked cmake @("--build", $Build, "--config", "Release", "--parallel", "4")
    $env:COLOSSUS_CEF_ROOT = $CefRoot
    $env:COLOSSUS_CEF_NATIVE_LIB_DIR = Join-Path $Build "Release"
    $Target = Join-Path $Repository ".local/cef-native-host-build-windows"
    Invoke-Checked cargo @("build", "--locked", "--manifest-path", "native/browser/driver/Cargo.toml", "--target-dir", $Target, "--lib")
    # The checkout may have shared write access. Component admission must retain
    # safe ancestry rather than inheriting the checkout's permissions.
    $UserProfile = [Environment]::GetFolderPath('UserProfile')
    if ([string]::IsNullOrWhiteSpace($UserProfile)) { throw "An owned native user profile is required for staging." }
    $StageParent = Join-Path $UserProfile "ColossusBrowserComponent-$([Guid]::NewGuid().ToString('N'))"
    Invoke-Checked python @("-B", "-c", 'import sys; from pathlib import Path; sys.path.insert(0, "native/browser/scripts"); from pki_fixture_private import private_directory; private_directory(Path(sys.argv[1]))', $StageParent)
    Write-Host "Private component allocation: $StageParent"
    $Stage = Join-Path $StageParent "component"
    # Cargo links debug/ and debug/deps/ to the same MSVC DLL. Give the stager an
    # independent copy; installed component verification still rejects aliases.
    $ClientCopy = Join-Path $Target "stage-client-$([Guid]::NewGuid().ToString('N')).dll"
    Copy-Item -LiteralPath (Join-Path $Target "debug/colossus_native_browser_host.dll") -Destination $ClientCopy
    try {
        $StageOutput = & python -B native/browser/scripts/stage_windows.py --host --cef-root $CefRoot --native-build $env:COLOSSUS_CEF_NATIVE_LIB_DIR --client-dll $ClientCopy --destination $Stage
        if ($LASTEXITCODE -ne 0) { throw "Private Windows browser host staging failed." }
        if ([string]::IsNullOrWhiteSpace($StageOutput)) { throw "Private Windows browser host staging returned no executable." }
        $Executable = $StageOutput.Trim()
    } finally { Remove-Item -LiteralPath $ClientCopy }
    Invoke-Checked python @("-B", "native/browser/scripts/component.py", "verify", "--root", $Stage)
    # Native diagnostics may select only this freshly built, verified false-mode
    # component. The profile and private channels still belong to the supervisor.
    $env:COLOSSUS_BROWSER_WINDOWS_ACCEPTANCE_COMPONENT = $Stage
    Write-Host "Private supervised host component: $Stage"
    Write-Host "Executable: $Executable"
} finally { Pop-Location }
