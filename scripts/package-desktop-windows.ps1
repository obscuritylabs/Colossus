param(
    [string]$ReleaseVersion = $env:COLOSSUS_DESKTOP_RELEASE_VERSION
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Fail([string]$Message) {
    throw "package-desktop-windows: $Message"
}

function Invoke-CheckedCommand([string]$Label, [string]$Command, [string[]]$Arguments) {
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) {
        Fail "$Label failed"
    }
}

function Detach-Executable([string]$Path) {
    $Detached = "$Path.colossus-detached"
    if (Test-Path -LiteralPath $Detached) {
        Fail "detached executable staging path already exists"
    }

    try {
        [IO.File]::Copy($Path, $Detached, $false)
        $SourceHash = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
        $DetachedHash = (Get-FileHash -LiteralPath $Detached -Algorithm SHA256).Hash
        if ($SourceHash -ne $DetachedHash) {
            Fail "detached executable does not match the built application"
        }
        [IO.File]::Move($Detached, $Path, $true)
    } finally {
        if (Test-Path -LiteralPath $Detached) {
            Remove-Item -LiteralPath $Detached -Force
        }
    }
}

$Repository = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$Desktop = Join-Path $Repository "apps/desktop"
$Native = Join-Path $Desktop "src-tauri"
$Target = if ($env:COLOSSUS_DESKTOP_TARGET) {
    $env:COLOSSUS_DESKTOP_TARGET
} else {
    "x86_64-pc-windows-msvc"
}
if ($Target -ne "x86_64-pc-windows-msvc") {
    Fail "the first Desktop preview supports only x86_64-pc-windows-msvc"
}
if ($env:COLOSSUS_DESKTOP_RELEASE_CHANNEL -notin @("developer_preview", "validation_only")) {
    Fail "unsigned Windows packaging is restricted to developer_preview or validation_only"
}
if ($env:COLOSSUS_DESKTOP_TEAM_ID -ne "UNSIGNED") {
    Fail "unsigned Windows packaging requires COLOSSUS_DESKTOP_TEAM_ID=UNSIGNED"
}
if (
    $ReleaseVersion -and
    $ReleaseVersion -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$'
) {
    Fail "desktop release version must be canonical semantic versioning"
}

$TargetRoot = if ($env:CARGO_TARGET_DIR) {
    if ([IO.Path]::IsPathRooted($env:CARGO_TARGET_DIR)) {
        $env:CARGO_TARGET_DIR
    } else {
        Join-Path $Repository $env:CARGO_TARGET_DIR
    }
} else {
    Join-Path $Native "target"
}
$ReleaseRoot = Join-Path $TargetRoot "$Target/release"
$Main = Join-Path $ReleaseRoot "colossus-desktop.exe"
$StagedSidecar = Join-Path $Native "binaries/colossus-sidecar-$Target.exe"
$StagedCli = Join-Path $Native "binaries/colossus-$Target.exe"
$Manifest = Join-Path $Native "binaries/colossus-bundle-manifest.json"
$Tauri = Join-Path $Desktop "node_modules/.bin/tauri.cmd"
$TypeScript = Join-Path $Desktop "node_modules/.bin/tsc.cmd"
$Vite = Join-Path $Desktop "node_modules/.bin/vite.cmd"
$TauriOverridePath = Join-Path `
    ([IO.Path]::GetTempPath()) `
    "colossus-tauri-override-$([Guid]::NewGuid().ToString('N')).json"

if (-not (Test-Path -LiteralPath $Tauri -PathType Leaf)) {
    Fail "run npm ci in apps/desktop before packaging"
}
if (-not (Test-Path -LiteralPath $TypeScript -PathType Leaf)) {
    Fail "run npm ci in apps/desktop before packaging"
}
if (-not (Test-Path -LiteralPath $Vite -PathType Leaf)) {
    Fail "run npm ci in apps/desktop before packaging"
}

$TauriOverride = [ordered]@{
    build = [ordered]@{
        beforeBuildCommand = "cmd /C echo Colossus desktop renderer build completed by package-desktop-windows.ps1"
    }
    bundle = [ordered]@{
        createUpdaterArtifacts = $false
    }
}
if ($ReleaseVersion) {
    $TauriOverride.version = $ReleaseVersion
}
$TauriOverrideJson = $TauriOverride | ConvertTo-Json -Compress -Depth 4
[IO.File]::WriteAllText(
    $TauriOverridePath,
    $TauriOverrideJson,
    [Text.UTF8Encoding]::new($false)
)

Push-Location $Repository
try {
    cargo xtask desktop prepare --profile release --target $Target
    if ($LASTEXITCODE -ne 0) { Fail "desktop binary preparation failed" }

    Push-Location $Desktop
    try {
        Invoke-CheckedCommand "renderer TypeScript app check" $TypeScript @("--noEmit", "-p", "tsconfig.app.json")
        Invoke-CheckedCommand "renderer TypeScript node check" $TypeScript @("--noEmit", "-p", "tsconfig.node.json")
        Invoke-CheckedCommand "renderer Vite build" $Vite @("build")
        Invoke-CheckedCommand `
            "renderer bundle size check" `
            "node" `
            @((Join-Path $Repository "scripts/check-desktop-renderer-bundle.mjs"), ".")
    } finally {
        Pop-Location
    }

    Push-Location $Desktop
    try {
        $BuildArguments = @(
            "build",
            "--target", $Target,
            "--no-sign",
            "--no-bundle",
            "--config", $TauriOverridePath
        )
        $BuildArguments += @("--", "--locked")
        & $Tauri @BuildArguments
        if ($LASTEXITCODE -ne 0) { Fail "Tauri application build failed" }
    } finally {
        Pop-Location
    }

    foreach ($Path in @($Main, $StagedSidecar, $StagedCli)) {
        if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
            Fail "expected release input is missing"
        }
    }

    # Cargo may hard-link the top-level Windows executable to its hashed artifact.
    # Replace it with an identical single-link file before the binding helper opens it.
    Detach-Executable $Main

    node (Join-Path $PSScriptRoot "write-desktop-bundle-manifest.mjs") `
        --target $Target `
        --release-channel $env:COLOSSUS_DESKTOP_RELEASE_CHANNEL `
        --sidecar $StagedSidecar `
        --cli $StagedCli `
        --output $Manifest
    if ($LASTEXITCODE -ne 0) { Fail "sealed bundle manifest generation failed" }

    node (Join-Path $PSScriptRoot "patch-desktop-manifest-binding.mjs") `
        --executable $Main `
        --manifest $Manifest
    if ($LASTEXITCODE -ne 0) { Fail "desktop manifest binding failed" }

    Push-Location $Desktop
    try {
        $BundleStartedAtUtc = [DateTime]::UtcNow
        $BundleArguments = @(
            "bundle",
            "--target", $Target,
            "--bundles", "nsis",
            "--no-sign",
            "--ci"
        )
        $BundleArguments += @("--config", $TauriOverridePath)
        & $Tauri @BundleArguments
        if ($LASTEXITCODE -ne 0) { Fail "NSIS packaging failed" }
    } finally {
        Pop-Location
    }

    $Installers = @(
        Get-ChildItem -LiteralPath (Join-Path $ReleaseRoot "bundle/nsis") `
            -Filter "*.exe" -File |
            Where-Object { $_.LastWriteTimeUtc -ge $BundleStartedAtUtc }
    )
    if ($Installers.Count -ne 1) {
        Fail "NSIS packaging must produce exactly one installer"
    }
    $Installer = $Installers[0]
    $InstallerSignature = "$($Installer.FullName).sig"
    if (Test-Path -LiteralPath $InstallerSignature) {
        Fail "unsigned Windows packaging unexpectedly created an updater signature"
    }
    $Checksum = (Get-FileHash -LiteralPath $Installer.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    "$Checksum  $($Installer.Name)" | Set-Content -LiteralPath "$($Installer.FullName).sha256" -Encoding ascii
    Copy-Item -LiteralPath $Manifest `
        -Destination (Join-Path $ReleaseRoot "colossus-bundle-manifest.json") -Force
    Write-Output $Installer.FullName
} finally {
    if (Test-Path -LiteralPath $Manifest -PathType Leaf) {
        Remove-Item -LiteralPath $Manifest -Force
    }
    if (Test-Path -LiteralPath $TauriOverridePath -PathType Leaf) {
        Remove-Item -LiteralPath $TauriOverridePath -Force
    }
    Pop-Location
}
