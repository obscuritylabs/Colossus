#requires -Version 7.0
<#
.SYNOPSIS
Creates, checks, or removes an isolated Docker-backed kind deployment.
.EXAMPLE
pwsh ./scripts/helm-local.ps1 -Action up -Name local -Kind C:/tools/kind.exe -Helm C:/tools/helm.exe
.EXAMPLE
pwsh ./scripts/helm-local.ps1 -Action down -Name local -RemoveFiles -WhatIf
#>
[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'Medium')]
param(
    [Parameter(Mandatory)][ValidateSet('up', 'test', 'down')][string] $Action,
    [ValidatePattern('^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$')][ValidateLength(1, 24)][string] $Name = 'local',
    [ValidateRange(1, 65535)][int] $Port = 443,
    [string] $Kind,
    [string] $Helm,
    [string] $Docker,
    [string] $Kubectl,
    [string] $Node,
    [string] $ControlPlaneImage,
    [string] $DocumentationImage,
    [switch] $LoadImages,
    [switch] $RemoveImages,
    [switch] $RemoveFiles
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'This harness requires Windows and PowerShell 7.' }
if (($RemoveFiles -or $RemoveImages) -and $Action -ne 'down') { throw 'Removal switches require -Action down.' }
if ($LoadImages -and $Action -ne 'up') { throw '-LoadImages requires -Action up.' }
. (Join-Path $PSScriptRoot 'helm-local/core.ps1')

$sourceRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$git = Resolve-HelmLocalTool 'git' ''
$common = (Invoke-HelmLocalNative $git @('-C', $sourceRoot, 'rev-parse', '--path-format=absolute', '--git-common-dir')).Trim()
if ([IO.Path]::GetFileName($common.TrimEnd('\', '/')) -ne '.git') { throw 'Expected a non-bare checkout with a .git common directory.' }
$primaryRoot = [IO.Path]::GetDirectoryName($common.TrimEnd('\', '/'))
$runRoot = [IO.Path]::GetFullPath((Join-Path $primaryRoot ".local/tests/helm-$Name"))
$cluster = "colossus-$Name"
$manifestPath = Join-Path $runRoot 'manifest.json'
Assert-HelmLocalBoundary $runRoot $primaryRoot
$manifest = Read-HelmLocalManifest $manifestPath $runRoot $cluster $primaryRoot

if ($Action -eq 'down' -and $null -eq $manifest) {
    Write-Output "No owned deployment exists at $runRoot. Nothing to remove."
    return
}
if ($Action -eq 'test' -and $null -eq $manifest) { throw 'No owned deployment exists. Run -Action up first.' }
if ($null -ne $manifest) {
    Assert-HelmLocalPrivateAcl $runRoot
    Assert-HelmLocalNoChildReparse $runRoot
    if ($Action -ne 'up') { $Port = [int]$manifest.port }
    elseif ([int]$manifest.port -ne $Port) { throw 'Changing the port requires down and a fresh deployment name/directory.' }
}

# A preview reads only local ownership and Git metadata. No client environment,
# directories, Docker commands, cluster operations, credentials, or files are changed.
if ($WhatIfPreference) {
    if ($Action -eq 'down') {
        [void]$PSCmdlet.ShouldProcess($cluster, 'Delete the exact owned kind cluster and its PostgreSQL data')
        if ($RemoveImages) {
            foreach ($image in @($manifest.images | Where-Object owned)) {
                [void]$PSCmdlet.ShouldProcess($image.reference, 'Remove image only after exact ID and ownership-label verification')
            }
        }
        if ($RemoveFiles) { [void]$PSCmdlet.ShouldProcess($runRoot, 'Remove the verified owned directory after resource cleanup') }
    } else { [void]$PSCmdlet.ShouldProcess($runRoot, "$Action local Helm deployment on port $Port") }
    return
}

if (-not $PSCmdlet.ShouldProcess($runRoot, "$Action local Helm deployment")) { return }
$tools = @{
    Docker = Resolve-HelmLocalTool 'docker' $Docker
    Kind = Resolve-HelmLocalTool 'kind' $Kind
}
if ($Action -ne 'down') {
    $tools.Helm = Resolve-HelmLocalTool 'helm' $Helm
    $tools.Kubectl = Resolve-HelmLocalTool 'kubectl' $Kubectl
    $tools.Curl = Resolve-HelmLocalTool 'curl.exe' ''
    $tools.Node = Resolve-HelmLocalTool 'node' $Node -Optional
}
$savedEnvironment = Save-HelmLocalEnvironment
try {
    if (-not (Test-Path -LiteralPath $runRoot)) {
        [void][IO.Directory]::CreateDirectory($runRoot)
        Set-HelmLocalPrivateAcl $runRoot
        # Establish ownership before prerequisite checks can fail, so a failed
        # first attempt remains inspectable and can be retried safely.
        $manifest = @{
            schemaVersion = 2; purpose = 'ColossusLocalHelmHarness'; rootPath = $runRoot; primaryRoot = $primaryRoot
            sourcePath = $sourceRoot; sourceCommit = (Invoke-HelmLocalNative $git @('-C', $sourceRoot, 'rev-parse', 'HEAD')).Trim()
            clusterName = $cluster; namespace = $cluster; port = $Port; images = @(); nodes = @()
        }
        Write-HelmLocalJson $manifestPath $manifest
    }
    Set-HelmLocalEnvironment $runRoot
    $engine = (Invoke-HelmLocalNative $tools.Docker @('info', '--format', '{{.OSType}}')).Trim()
    if ($engine -cne 'linux') { throw 'Docker Desktop must be running its Linux engine.' }
    if ($Action -eq 'down') {
        Remove-HelmLocalDeployment $tools $manifest $runRoot $primaryRoot -RemoveImages:$RemoveImages -RemoveFiles:$RemoveFiles
        return
    }
    if ($Action -eq 'up') {
        $manifest = Start-HelmLocalDeployment $tools $sourceRoot $primaryRoot $runRoot $cluster $Name $Port $manifest $ControlPlaneImage $DocumentationImage -LoadImages:$LoadImages
    }
    Test-HelmLocalDeployment $tools $manifest $runRoot
} finally {
    Restore-HelmLocalEnvironment $savedEnvironment
}
