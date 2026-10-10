param([switch]$BuildOnly)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
if (-not $IsWindows -or [Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne "X64") {
    throw "The embedded Chromium developer launcher requires PowerShell 7 on Windows x64."
}
function Invoke-Checked([string]$Command, [string[]]$Arguments) {
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) { throw "Chromium developer command failed: $Command" }
}
$Repository = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Desktop = Join-Path $Repository "apps/desktop"
$Native = Join-Path $Desktop "src-tauri"
Push-Location $Repository
try {
    foreach ($Command in @("python", "cmake", "cargo", "rustc", "npm", "node")) {
        $null = Get-Command $Command -ErrorAction Stop
    }
    $HostTriple = (& rustc --print host-tuple).Trim()
    if ($LASTEXITCODE -ne 0 -or $HostTriple -ne "x86_64-pc-windows-msvc") {
        throw "Use the native Windows x64 MSVC Rust toolchain."
    }
    $CefRoot = (& python -B native/browser/scripts/component.py fetch --platform windows64).Trim()
    if ($LASTEXITCODE -ne 0) { throw "Pinned CEF acquisition failed." }
    $Build = Join-Path $Repository ".local/cef-build-windows64"
    Invoke-Checked cmake @("-S", "native/browser", "-B", $Build, "-A", "x64", "-DCEF_ROOT=$CefRoot", "-DCEF_RUNTIME_LIBRARY_FLAG=/MD", "-DUSE_SANDBOX=ON")
    Invoke-Checked cmake @("--build", $Build, "--config", "Release", "--parallel", "4")
    $env:COLOSSUS_CEF_ROOT = $CefRoot
    $env:COLOSSUS_CEF_NATIVE_LIB_DIR = Join-Path $Build "Release"
    Write-Host "COLOSSUS_CEF_ROOT=$CefRoot"
    Write-Host "COLOSSUS_CEF_NATIVE_LIB_DIR=$env:COLOSSUS_CEF_NATIVE_LIB_DIR"
    Invoke-Checked node @("scripts/development-launch.mjs", "--", "cargo", "xtask", "desktop", "prepare", "--profile", "debug")
    # Build the client DLL explicitly: a plain Tauri .exe cannot carry CEF's
    # bootstrap-owned sandbox context. Embed the built renderer in this DLL.
    if (-not $env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = Join-Path $Native "target" }
    if (-not [IO.Path]::IsPathRooted($env:CARGO_TARGET_DIR)) {
        $env:CARGO_TARGET_DIR = Join-Path $Repository $env:CARGO_TARGET_DIR
    }
    Push-Location $Desktop
    try {
        Invoke-Checked npm @("ci", "--ignore-scripts")
        Invoke-Checked npm @("run", "build")
    } finally { Pop-Location }
    Invoke-Checked cargo @("build", "--locked", "--manifest-path", (Join-Path $Native "Cargo.toml"), "--lib", "--features", "embedded-chromium-preview,tauri/custom-protocol")
    $Stage = Join-Path $Repository ".local/cef-desktop-preview-windows-$([Guid]::NewGuid().ToString('N'))"
    $Executable = (& python -B native/browser/scripts/stage_windows.py --cef-root $CefRoot --native-build $env:COLOSSUS_CEF_NATIVE_LIB_DIR --client-dll (Join-Path $env:CARGO_TARGET_DIR "debug/colossus_desktop_lib.dll") --destination $Stage --dictation-resources (Join-Path $Native "dictation-assets")).Trim()
    if ($LASTEXITCODE -ne 0) { throw "Windows CEF staging failed." }
    Invoke-Checked python @("-B", "native/browser/scripts/component.py", "verify", "--root", $Stage)
    Write-Host "Chromium developer component: $Stage"
    if (-not $BuildOnly) { Invoke-Checked node @("scripts/development-launch.mjs", "--", $Executable) }
} finally { Pop-Location }
