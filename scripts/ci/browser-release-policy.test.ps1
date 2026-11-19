# Execute the actual publisher admission guards without building or launching a browser.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$Repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))

function Read-Guard([string]$Relative, [string]$Needle, [switch]$WholeBlock, [string]$SecondCondition = '') {
    $tokens = $null
    $errors = $null
    $ast = [Management.Automation.Language.Parser]::ParseFile(
        (Join-Path $Repository $Relative), [ref]$tokens, [ref]$errors)
    if (@($errors).Count) { throw "Cannot parse publisher admission: $Relative" }
    $guards = @($ast.FindAll({
        param($node)
        $node -is [Management.Automation.Language.IfStatementAst] -and
            $node.Clauses[0].Item1.Extent.Text.Contains($Needle, [StringComparison]::Ordinal) -and
            (!$SecondCondition -or ($node.Clauses.Count -eq 2 -and
                $node.Clauses[1].Item1.Extent.Text.Contains($SecondCondition, [StringComparison]::Ordinal)))
    }, $true))
    if ($guards.Count -ne 1) { throw "Publisher admission is absent or ambiguous: $Relative / $Needle" }
    $text = if ($WholeBlock) { $guards[0].Extent.Text } else { $guards[0].Clauses[0].Item1.Extent.Text }
    [scriptblock]::Create($text)
}

function Assert-Rejection($Guard, [bool]$Expected, [string]$Case) {
    $actual = & $Guard
    if ($actual -isnot [bool] -or $actual -ne $Expected) {
        throw "Publisher admission changed: $Case"
    }
}

function Assert-PhaseAdmission($Guard, [bool]$Rejected, [string]$Case) {
    $failed = $false
    try { & $Guard } catch { $failed = $true }
    if ($failed -ne $Rejected) { throw "Publisher phase admission changed: $Case" }
}

# Replace the guards' failure and filesystem predicates with fixture operations.
# Path composition stays real; no publisher build, file access or signing runs.
function Fail([string]$Message) { throw $Message }
function Test-Path([string]$LiteralPath, [string]$PathType) {
    if ($LiteralPath -ne 'C:\ColossusPolicyFixture\component\browser-component.json' -or $PathType -ne 'Leaf') {
        throw 'Publisher admission requested an unexpected fixture path'
    }
    $script:ManifestExists
}

$stage = 'scripts/stage-browser-windows-preview.ps1'
$package = 'scripts/package-desktop-windows.ps1'
$stageChannel = Read-Guard $stage '$env:COLOSSUS_DESKTOP_RELEASE_CHANNEL -notin'
$stageSigning = Read-Guard $stage "`$Phase -eq 'seal'"
$packageChannel = Read-Guard $package '$env:COLOSSUS_DESKTOP_RELEASE_CHANNEL -notin'
# Select the complete unsigned/signed admission pair, not the later NSIS
# argument-selection branch that also tests Phase=all.
$packagePhase = Read-Guard $package '$Phase -eq "all"' -WholeBlock -SecondCondition '$env:COLOSSUS_DESKTOP_RELEASE_CHANNEL -notin'
$sealFile = Read-Guard $package '$PreviewSealInfo.Length -le 0'
$sealBinding = Read-Guard $package '$PreviewSeal.component_manifest_sha256 -cne'
$saved = @{}
foreach ($name in @('COLOSSUS_DESKTOP_RELEASE_CHANNEL', 'COLOSSUS_DESKTOP_TEAM_ID',
        'COLOSSUS_SIDECAR_WINDOWS_BROWSER_FUNCTIONAL_PREVIEW', 'COLOSSUS_BROWSER_PRESENTER_NATIVE_LIB_DIR')) {
    $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
try {
    $BrowserComponent = 'C:\ColossusPolicyFixture\component'
    $env:COLOSSUS_SIDECAR_WINDOWS_BROWSER_FUNCTIONAL_PREVIEW = 'C:\ColossusPolicyFixture\seal.json'
    $env:COLOSSUS_BROWSER_PRESENTER_NATIVE_LIB_DIR = 'C:\ColossusPolicyFixture\presenter'
    $script:ManifestExists = $true
    foreach ($channel in @('stable', 'developer_preview', 'validation_only', 'unknown', '')) {
        $env:COLOSSUS_DESKTOP_RELEASE_CHANNEL = $channel
        $known = $channel -in @('stable', 'developer_preview', 'validation_only')
        Assert-Rejection $stageChannel (!$known) "stage channel '$channel'"
        Assert-Rejection $packageChannel (!$known) "package channel '$channel'"
        foreach ($Phase in @('build', 'seal')) {
            foreach ($RequireSignatures in @($false, $true)) {
                $signed = $channel -in @('stable', 'developer_preview')
                $rejected = $signed -and $Phase -eq 'seal' -and !$RequireSignatures
                Assert-Rejection $stageSigning $rejected "stage $channel/$Phase/signatures=$RequireSignatures"
            }
        }
        foreach ($Phase in @('all', 'build', 'bind', 'bundle', 'finalize')) {
            foreach ($team in @('UNSIGNED', 'OBSCURITY_LABS_LLC', 'OTHER')) {
                $env:COLOSSUS_DESKTOP_TEAM_ID = $team
                $allowed = if ($Phase -eq 'all') {
                    $channel -eq 'validation_only' -and $team -eq 'UNSIGNED'
                } else {
                    $channel -in @('stable', 'developer_preview') -and $team -eq 'OBSCURITY_LABS_LLC'
                }
                Assert-PhaseAdmission $packagePhase (!$allowed) "package $channel/$Phase/$team"
            }
        }
    }

    # Published channels must still reject missing compiler-bound browser inputs.
    foreach ($channel in @('stable', 'developer_preview')) {
        $env:COLOSSUS_DESKTOP_RELEASE_CHANNEL = $channel
        $script:ManifestExists = $false
        Assert-Rejection $packageChannel $true "$channel missing component inventory"
        $script:ManifestExists = $true
        $BrowserComponent = 'relative/component'
        Assert-Rejection $packageChannel $true "$channel relative component"
        $BrowserComponent = 'C:\ColossusPolicyFixture\component'
        $env:COLOSSUS_SIDECAR_WINDOWS_BROWSER_FUNCTIONAL_PREVIEW = ''
        Assert-Rejection $packageChannel $true "$channel missing compiler seal"
        $env:COLOSSUS_SIDECAR_WINDOWS_BROWSER_FUNCTIONAL_PREVIEW = 'C:\ColossusPolicyFixture\seal.json'
        $env:COLOSSUS_BROWSER_PRESENTER_NATIVE_LIB_DIR = ''
        Assert-Rejection $packageChannel $true "$channel missing native presenter"
        $env:COLOSSUS_BROWSER_PRESENTER_NATIVE_LIB_DIR = 'C:\ColossusPolicyFixture\presenter'
    }
    foreach ($length in @(0, 1, 4096, 4097)) {
        $PreviewSealInfo = [pscustomobject]@{ Length = $length; Attributes = [IO.FileAttributes]::Normal }
        Assert-Rejection $sealFile ($length -in @(0, 4097)) "seal length $length"
    }
    $PreviewSealInfo = [pscustomobject]@{ Length = 100; Attributes = [IO.FileAttributes]::ReparsePoint }
    Assert-Rejection $sealFile $true 'linked compiler seal'
    $PreviewDigest = 'a' * 64
    $PreviewSeal = [pscustomobject]@{ component_manifest_sha256 = $PreviewDigest; component_path = 'browser/component' }
    Assert-Rejection $sealBinding $false 'exact inventory digest and relative location'
    $PreviewSeal.component_manifest_sha256 = 'A' * 64
    Assert-Rejection $sealBinding $true 'case-changed inventory digest'
    $PreviewSeal.component_manifest_sha256 = $PreviewDigest
    $PreviewSeal.component_path = '../browser/component'
    Assert-Rejection $sealBinding $true 'changed installation-relative location'
    Write-Output 'PASS stable and preview browser admission preserves signing and sealed inputs'
} finally {
    foreach ($name in $saved.Keys) {
        [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process')
    }
}
