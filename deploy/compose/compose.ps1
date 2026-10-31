#requires -Version 7.2
<#
.SYNOPSIS
Install and operate one release-matched Colossus Docker Compose deployment.
.EXAMPLE
pwsh ./deploy/compose/compose.ps1 -Action up -Name demo
.EXAMPLE
pwsh ./deploy/compose/compose.ps1 -Action down -Name demo
#>
[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'Medium')]
param(
    [Parameter(Mandatory)][ValidateSet('init','up','status','test','down','upgrade','backup','restore')][string] $Action,
    [ValidatePattern('^[a-z][a-z0-9-]{0,23}$')][string] $Name = 'local',
    [string] $StateDirectory,
    [string] $ReleaseTag,
    [string] $HostName = 'localhost',
    [string] $BindAddress = '127.0.0.1',
    [ValidateRange(1,65535)][int] $Port = 9443,
    [ValidateRange(1,65535)][int] $ConnectorPort = 9444,
    [switch] $CoreOnly,
    [switch] $LocalImages,
    [switch] $RemoveData,
    [string] $Docker = 'docker',
    [string] $Node = 'node',
    [string] $Backup
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'lib/private.ps1')
. (Join-Path $PSScriptRoot 'lib/material.ps1')
. (Join-Path $PSScriptRoot 'lib/docker.ps1')
. (Join-Path $PSScriptRoot 'lib/backup.ps1')
. (Join-Path $PSScriptRoot 'lib/bootstrap.ps1')
if ($RemoveData -and $Action -cne 'down') { throw '-RemoveData requires -Action down.' }
if ($Backup -and $Action -cnotin @('backup','restore')) { throw '-Backup requires backup or restore.' }
$root = Get-ComposeRoot $Name $StateDirectory
$script:ComposeFailureRoot = $null
$manifestPath = Join-Path $root 'manifest.json'
Assert-ComposePath $root
$manifest = $null
if (Test-Path -LiteralPath $root) {
    Assert-ComposePrivateDirectory $root
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) { throw 'An existing directory has no deployment receipt; refusing adoption.' }
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json -AsHashtable
    if (-not $manifest.ContainsKey('initialized') -or -not $manifest.initialized) { throw 'Initialization was interrupted. Retain the partial allocation for inspection; use a fresh name.' }
    Assert-ComposeManifest $root $manifest
    if ($manifest.ContainsKey('restoring') -and $manifest.restoring -and $Action -cnotin @('down','status')) { throw 'Restore was interrupted. Retain its evidence and restore the backup into a fresh name; incomplete state will not be started.' }
    if ($Action -ceq 'restore' -or $Action -ceq 'init') { throw 'Initialization and restore require a new deployment name.' }
    if ($PSBoundParameters.ContainsKey('CoreOnly') -and $CoreOnly.IsPresent -ne ($manifest.profile -ceq 'core')) { throw 'Changing the chat profile requires a fresh deployment name.' }
    if ($PSBoundParameters.ContainsKey('LocalImages') -and $LocalImages.IsPresent -ne ($manifest.ContainsKey('localImages') -and $manifest.localImages)) { throw 'Local image selection cannot change implicitly on a retained deployment.' }
    foreach ($binding in @{ HostName='hostname'; BindAddress='bindAddress'; Port='browserPort'; ConnectorPort='connectorPort' }.GetEnumerator()) {
        if ($PSBoundParameters.ContainsKey($binding.Key) -and (Get-Variable -Name $binding.Key -ValueOnly) -cne $manifest[$binding.Value]) { throw 'Retained hostname and port bindings cannot be changed implicitly.' }
    }
    if ($PSBoundParameters.ContainsKey('ReleaseTag') -and $Action -cne 'upgrade' -and $ReleaseTag -cne $manifest.releaseTag) { throw 'Use -Action upgrade to select a different release.' }
} elseif ($Action -cnotin @('init','up','restore')) { throw 'No deployment exists with this name.' }
$description = if ($RemoveData) { 'Remove this exact owned Compose deployment AND its persistent database volumes; retain credential files' } else { "$Action Colossus Compose deployment" }
if (-not $PSCmdlet.ShouldProcess($root, $description)) { return }
if ($null -eq $manifest -and $Action -cne 'restore') {
    if (-not $ReleaseTag) {
        $chart = @((Join-Path $PSScriptRoot '../control-plane/chart/Chart.yaml'),(Join-Path $PSScriptRoot '../chart/Chart.yaml')) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
        if (-not $chart) { throw 'Pass -ReleaseTag when the matching chart is unavailable.' }
        $ReleaseTag = 'v' + [regex]::Match([IO.File]::ReadAllText($chart), '(?m)^appVersion:\s*"?([^"\s]+)').Groups[1].Value
    }
    $manifest = Initialize-ComposeDeployment $root $PSScriptRoot $Name $ReleaseTag $HostName $BindAddress $Port $ConnectorPort -CoreOnly:$CoreOnly -LocalImages:$LocalImages
}
if ($Action -ceq 'init') { Write-Output "Initialized $root. Credentials are in admin-login.txt; no containers were started."; return }
$dockerCommand = Get-Command $Docker -CommandType Application -ErrorAction Stop | Select-Object -First 1
$Docker = $dockerCommand.Source
$lock = $null
try {
    if ($Action -ceq 'restore') { $script:ComposeFailureRoot = $root }
    if ($Action -ceq 'restore') { $manifest = Restore-ComposeBackup $Docker $root $Name $Backup ([ref]$lock) }
    $script:ComposeFailureRoot = $root
    Assert-ComposeManifest $root $manifest
    if ($null -eq $lock) {
        Assert-ComposePath (Join-Path $root '.operation.lock')
        $lock = [IO.File]::Open((Join-Path $root '.operation.lock'), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    }
    Assert-ComposeEngine $Docker $root $manifest
    switch ($Action) {
        'up' { Start-ComposeDeployment $Docker $root $manifest }
        'restore' { Start-ComposeDeployment $Docker $root $manifest }
        'status' { $null = Get-ComposeResources $Docker $manifest; Write-Output (Invoke-Compose $Docker $root $manifest @('ps','--all')) }
        'down' { Stop-ComposeDeployment $Docker $root $manifest -RemoveData:$RemoveData; Write-Output $(if ($RemoveData) { 'Owned containers and data removed. Credential allocation retained.' } else { 'Containers stopped. Database volumes and credentials retained.' }) }
        'test' {
            $null = Get-ComposeResources $Docker $manifest
            $nodeCommand = Get-Command $Node -CommandType Application -ErrorAction Stop | Select-Object -First 1
            & $nodeCommand.Source (Join-Path $PSScriptRoot 'probe.cjs') $root
            if ($LASTEXITCODE -ne 0) { throw 'Compose acceptance failed; private verification receipt retained.' }
        }
        'upgrade' {
            Assert-ComposeUpgrade $manifest.releaseTag $ReleaseTag
            $null = Get-ComposeResources $Docker $manifest
            # Database migrations may be irreversible; always retain a quiesced backup.
            Backup-ComposeDeployment $Docker $root $manifest $Backup -KeepStopped
            $manifest['previousImages'] = @{ server=$manifest.serverImage; documentation=$manifest.documentationImage; releaseTag=$manifest.releaseTag }
            $manifest.releaseTag = $ReleaseTag
            $manifest.serverImage = "ghcr.io/obscuritylabs/colossus-control-plane:$ReleaseTag"
            $manifest.documentationImage = "ghcr.io/obscuritylabs/colossus-documentation:$ReleaseTag"
            Start-ComposeDeployment $Docker $root $manifest
        }
        'backup' { Backup-ComposeDeployment $Docker $root $manifest $Backup }
    }
    if ($Action -cin @('up','restore','upgrade')) {
        Initialize-ComposeHostedProject $root $manifest
        Write-Output "Ready: https://$($manifest.hostname):$($manifest.browserPort)"
        Write-Output "Credentials: $(Join-Path $root 'admin-login.txt')"
        Write-Output "Browser trust certificate: $(Join-Path $root 'browser-ca.pem')"
        Write-Output "Connector enrollment: https://$($manifest.hostname):$($manifest.browserPort)/api/enroll"
        if ($manifest.profile -ceq 'hosted-demo') { Write-Output 'Hosted demo enabled. Its anchor shares this Docker host; independent production custody is not established.' }
    }
} finally { if ($null -ne $lock) { $lock.Dispose() } }
