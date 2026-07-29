param(
    [string]$ReleaseVersion = $env:COLOSSUS_DESKTOP_RELEASE_VERSION
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Fail([string]$Message) {
    throw "package-desktop-windows: $Message"
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

if (-not (Test-Path -LiteralPath $Tauri -PathType Leaf)) {
    Fail "run npm ci in apps/desktop before packaging"
}

Push-Location $Repository
try {
    cargo xtask desktop prepare --profile release --target $Target
    if ($LASTEXITCODE -ne 0) { Fail "desktop binary preparation failed" }

    Push-Location $Desktop
    try {
        $BuildArguments = @("build", "--target", $Target, "--no-sign", "--no-bundle")
        $VersionOverride = $null
        if ($ReleaseVersion) {
            $VersionOverride = "{`"version`":`"$ReleaseVersion`"}"
            $BuildArguments += @("--config", $VersionOverride)
        }
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
        $BundleArguments = @(
            "bundle",
            "--target", $Target,
            "--bundles", "nsis",
            "--no-sign",
            "--ci"
        )
        $BundleConfiguration = [ordered]@{}
        if ($VersionOverride) {
            $BundleConfiguration.version = $ReleaseVersion
        }
        $BundleConfiguration.bundle = [ordered]@{
            createUpdaterArtifacts = $false
        }
        if ($BundleConfiguration.Count -gt 0) {
            $BundleOverride = $BundleConfiguration | ConvertTo-Json -Compress -Depth 4
            $BundleArguments += @("--config", $BundleOverride)
        }
        & $Tauri @BundleArguments
        if ($LASTEXITCODE -ne 0) { Fail "NSIS packaging failed" }
    } finally {
        Pop-Location
    }

    $Installers = @(
        Get-ChildItem -LiteralPath (Join-Path $ReleaseRoot "bundle/nsis") `
            -Filter "*.exe" -File
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
    Pop-Location
}
