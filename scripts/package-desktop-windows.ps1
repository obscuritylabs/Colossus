param(
    [string]$ReleaseVersion = $env:COLOSSUS_DESKTOP_RELEASE_VERSION,
    [ValidateSet("all", "build", "bind", "bundle", "finalize")]
    [string]$Phase = "all"
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
    Fail "Windows Desktop packaging currently supports only x86_64-pc-windows-msvc"
}
if ($Phase -eq "all") {
    if ($env:COLOSSUS_DESKTOP_RELEASE_CHANNEL -ne "validation_only" -or
        $env:COLOSSUS_DESKTOP_TEAM_ID -ne "UNSIGNED") {
        Fail "unsigned Windows packaging is restricted to validation_only with COLOSSUS_DESKTOP_TEAM_ID=UNSIGNED"
    }
} elseif ($env:COLOSSUS_DESKTOP_RELEASE_CHANNEL -notin @("stable", "developer_preview") -or
    $env:COLOSSUS_DESKTOP_TEAM_ID -ne "OBSCURITY_LABS_LLC") {
    Fail "signed Windows packaging requires stable or developer_preview with COLOSSUS_DESKTOP_TEAM_ID=OBSCURITY_LABS_LLC"
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
$StagedRipgrep = Join-Path $Native "binaries/rg-$Target.exe"
$BrowserPreview = $env:COLOSSUS_DESKTOP_BROWSER_FUNCTIONAL_PREVIEW -eq '1'
$BrowserComponent = $env:COLOSSUS_BROWSER_PREVIEW_COMPONENT
if ($BrowserPreview) {
    if ($env:COLOSSUS_DESKTOP_RELEASE_CHANNEL -notin @('developer_preview', 'validation_only') -or
        -not $BrowserComponent -or -not [IO.Path]::IsPathFullyQualified($BrowserComponent) -or
        -not (Test-Path -LiteralPath (Join-Path $BrowserComponent 'browser-component.json') -PathType Leaf) -or
        -not $env:COLOSSUS_SIDECAR_WINDOWS_BROWSER_FUNCTIONAL_PREVIEW -or
        -not $env:COLOSSUS_BROWSER_PRESENTER_NATIVE_LIB_DIR) {
        Fail 'functional-preview packaging requires its sealed relative descriptor, component, and native presenter'
    }
    & python -B (Join-Path $Repository 'native/browser/scripts/component.py') verify --root $BrowserComponent
    if ($LASTEXITCODE -ne 0) { Fail 'functional-preview component verification failed' }
    $PreviewSealPath = $env:COLOSSUS_SIDECAR_WINDOWS_BROWSER_FUNCTIONAL_PREVIEW
    $PreviewSealInfo = Get-Item -LiteralPath $PreviewSealPath
    if ($PreviewSealInfo.Length -le 0 -or $PreviewSealInfo.Length -gt 4096 -or
        ($PreviewSealInfo.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        Fail 'functional-preview compiler seal is not one bounded regular file'
    }
    $PreviewSeal = Get-Content -LiteralPath $PreviewSealPath -Raw | ConvertFrom-Json
    $PreviewDigest = (Get-FileHash -LiteralPath (Join-Path $BrowserComponent 'browser-component.json') -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($PreviewSeal.component_manifest_sha256 -cne $PreviewDigest -or
        $PreviewSeal.component_path -cne 'browser/component') {
        Fail 'functional-preview compiler seal differs from the packaged component'
    }
}
$Manifest = Join-Path $Native "binaries/colossus-bundle-manifest.json"
$Tauri = Join-Path $Desktop "node_modules/.bin/tauri.cmd"
$TypeScript = Join-Path $Desktop "node_modules/.bin/tsc.cmd"
$Vite = Join-Path $Desktop "node_modules/.bin/vite.cmd"
$TauriOverridePath = Join-Path `
    ([IO.Path]::GetTempPath()) `
    "colossus-tauri-override-$([Guid]::NewGuid().ToString('N')).json"

$MetadataJson = & cargo metadata --locked --no-deps --format-version 1 `
    --manifest-path (Join-Path $Repository "Cargo.toml")
if ($LASTEXITCODE -ne 0) {
    Fail "workspace metadata inspection failed"
}
$Metadata = $MetadataJson | ConvertFrom-Json
$CliPackages = @($Metadata.packages | Where-Object { $_.name -eq "colossus-cli" })
if ($CliPackages.Count -ne 1) {
    Fail "workspace metadata must contain exactly one colossus-cli package"
}
$BundledVersion = [string]$CliPackages[0].version
if (-not $ReleaseVersion) {
    $ReleaseVersion = $BundledVersion
}
if (
    $ReleaseVersion -notmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$'
) {
    Fail "desktop release version must be canonical semantic versioning"
}
if ($ReleaseVersion -ne $BundledVersion) {
    Fail "desktop release version $ReleaseVersion must match bundled CLI version $BundledVersion"
}

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
        externalBin = @("binaries/colossus-sidecar", "binaries/colossus", "binaries/rg")
        resources = [ordered]@{
            "dictation-assets/ggml-tiny.en.bin" = "dictation/ggml-tiny.en.bin"
            "dictation-assets/models.json" = "dictation/models.json"
            "dictation-assets/LICENSE-MIT" = "dictation/LICENSE-MIT"
            "binaries/colossus-bundle-manifest.json" = "colossus-bundle-manifest.json"
            "binaries/COPYING" = "ripgrep/COPYING"
            "binaries/LICENSE-MIT" = "ripgrep/LICENSE-MIT"
            "binaries/UNLICENSE" = "ripgrep/UNLICENSE"
        }
    }
}
if ($BrowserPreview) {
    # The signed sidecar discovers only this fixed installation-relative location.
    # Source paths exist solely in the publisher build configuration.
    $TauriOverride.bundle.resources[$BrowserComponent.Replace('\', '/') + '/'] = 'browser/component/'
}
if ($Phase -eq "bundle") {
    $TauriOverride.bundle["windows"] = [ordered]@{
        signCommand = [ordered]@{
            cmd = "pwsh"
            args = @(
                "-NoProfile",
                "-NonInteractive",
                "-File",
                (Join-Path $PSScriptRoot "ci/sign-tauri-windows.ps1"),
                "%1"
            )
        }
    }
}
if ($BrowserPreview) {
    if (-not $TauriOverride.bundle.Contains('windows')) { $TauriOverride.bundle['windows'] = [ordered]@{} }
    $TauriOverride.bundle.windows['nsis'] = [ordered]@{
        installerHooks = (Join-Path $Native 'installer-browser-preview-hooks.nsh')
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
    if ($Phase -in @("all", "build")) {
        node (Join-Path $PSScriptRoot "stage-dictation-model.mjs")
        if ($LASTEXITCODE -ne 0) { Fail "dictation model staging failed" }
        cargo xtask desktop prepare --profile release --target $Target
        if ($LASTEXITCODE -ne 0) { Fail "desktop binary preparation failed" }
        if ($BrowserPreview) {
            # Compile after component signing/inventory; the sidecar independently
            # seals the exact digest before its own publisher signature is applied.
            cargo build --locked --release --package colossus-sidecar --bin colossus-sidecar `
                --target $Target --target-dir $TargetRoot --features windows-browser-functional-preview
            if ($LASTEXITCODE -ne 0) { Fail 'functional-preview sidecar build failed' }
            Copy-Item -LiteralPath (Join-Path $TargetRoot "$Target/release/colossus-sidecar.exe") -Destination $StagedSidecar -Force
        }
        node (Join-Path $PSScriptRoot "stage-ripgrep.mjs") `
            --target $Target `
            --output (Join-Path $Native "binaries") `
            --binary-name "rg-$Target.exe"
        if ($LASTEXITCODE -ne 0) { Fail "ripgrep staging failed" }

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
            if ($BrowserPreview) { $BuildArguments += @('--features', 'windows-browser-functional-preview') }
            $BuildArguments += @("--", "--locked")
            & $Tauri @BuildArguments
            if ($LASTEXITCODE -ne 0) { Fail "Tauri application build failed" }
        } finally {
            Pop-Location
        }

        foreach ($Path in @($Main, $StagedSidecar, $StagedCli, $StagedRipgrep)) {
            if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
                Fail "expected release input is missing"
            }
        }

        # Cargo may hard-link the top-level Windows executable to its hashed artifact.
        # Replace it with an identical single-link file before the binding helper opens it.
        Detach-Executable $Main
    }

    if ($Phase -eq "build") {
        Write-Output $Main
        return
    }

    foreach ($Path in @($Main, $StagedSidecar, $StagedCli, $StagedRipgrep)) {
        if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
            Fail "expected release input is missing"
        }
    }

    if ($Phase -eq "bind") {
        foreach ($Path in @($StagedSidecar, $StagedCli, $StagedRipgrep)) {
            & (Join-Path $PSScriptRoot "ci/verify-authenticode.ps1") -Path $Path
        }
        if ((Get-AuthenticodeSignature -LiteralPath $Main).Status -ne "NotSigned") {
            Fail "desktop executable must be unsigned until after manifest binding"
        }
    }

    if ($Phase -in @("all", "bind")) {
        node (Join-Path $PSScriptRoot "write-desktop-bundle-manifest.mjs") `
            --target $Target `
            --release-channel $env:COLOSSUS_DESKTOP_RELEASE_CHANNEL `
            --sidecar $StagedSidecar `
            --cli $StagedCli `
            --ripgrep $StagedRipgrep `
            --output $Manifest
        if ($LASTEXITCODE -ne 0) { Fail "sealed bundle manifest generation failed" }

        node (Join-Path $PSScriptRoot "patch-desktop-manifest-binding.mjs") `
            --executable $Main `
            --manifest $Manifest
        if ($LASTEXITCODE -ne 0) { Fail "desktop manifest binding failed" }
    }

    if ($Phase -eq "bind") {
        Write-Output $Main
        return
    }

    if ($Phase -in @("bundle", "finalize")) {
        foreach ($Path in @($StagedSidecar, $StagedCli, $StagedRipgrep)) {
            & (Join-Path $PSScriptRoot "ci/verify-authenticode.ps1") -Path $Path
        }
    }
    if ($Phase -eq "bundle" -and
        (Get-AuthenticodeSignature -LiteralPath $Main).Status -ne "NotSigned") {
        Fail "Tauri must patch the unsigned desktop executable before signing it"
    }

    if ($Phase -in @("all", "bundle")) {

        Push-Location $Desktop
        try {
            $BundleStartedAtUtc = [DateTime]::UtcNow
            $BundleArguments = @(
                "bundle",
                "--target", $Target,
                "--bundles", "nsis",
                "--ci"
            )
            if ($Phase -eq "all") {
                $BundleArguments += "--no-sign"
            }
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
        if ($Phase -eq "bundle") {
            & (Join-Path $PSScriptRoot "ci/verify-authenticode.ps1") -Path $Installer.FullName
        }
    }

    if ($Phase -eq "bundle") {
        Write-Output $Installer.FullName
        return
    }

    if ($Phase -eq "finalize") {
        $Installers = @(Get-ChildItem -LiteralPath (Join-Path $ReleaseRoot "bundle/nsis") -Filter "*.exe" -File)
        if ($Installers.Count -ne 1) {
            Fail "NSIS packaging must produce exactly one installer"
        }
        $Installer = $Installers[0]
        & (Join-Path $PSScriptRoot "ci/verify-authenticode.ps1") -Path $Installer.FullName
    }
    $InstallerSignature = "$($Installer.FullName).sig"
    if (Test-Path -LiteralPath $InstallerSignature) {
        Fail "Windows packaging unexpectedly created an updater signature"
    }
    $Checksum = (Get-FileHash -LiteralPath $Installer.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    "$Checksum  $($Installer.Name)" | Set-Content -LiteralPath "$($Installer.FullName).sha256" -Encoding ascii
    Copy-Item -LiteralPath $Manifest `
        -Destination (Join-Path $ReleaseRoot "colossus-bundle-manifest.json") -Force
    Write-Output $Installer.FullName
} finally {
    if ($Phase -in @("all", "finalize") -and (Test-Path -LiteralPath $Manifest -PathType Leaf)) {
        Remove-Item -LiteralPath $Manifest -Force
    }
    if (Test-Path -LiteralPath $TauriOverridePath -PathType Leaf) {
        Remove-Item -LiteralPath $TauriOverridePath -Force
    }
    Pop-Location
}
