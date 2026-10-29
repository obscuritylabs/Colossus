Set-StrictMode -Version Latest

function Get-HelmLocalAllocationPath {
    param([string] $Cluster, [string] $DeploymentId)
    $match = [regex]::Match($Cluster, '^colossus-([a-z][a-z0-9]*(?:-[a-z0-9]+)*)$')
    if (-not $match.Success -or $match.Groups[1].Length -gt 24 -or $DeploymentId -cnotmatch '^[a-f0-9]{32}$') { throw 'Invalid private allocation identity.' }
    $name = $match.Groups[1].Value
    # Do not use a caller-controlled HOME override for credential custody.
    $userProfileRoot = [Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)
    return [IO.Path]::GetFullPath((Join-Path $userProfileRoot ".colossus-tests/helm-$name-$DeploymentId"))
}

function New-HelmLocalManifest {
    param([string] $Root, [string] $Primary, [string] $Source, [string] $Cluster, [int] $Port, [string] $SourceCommit)
    $id = [Guid]::NewGuid().ToString('N')
    return @{
        schemaVersion = 3; profile = 'full-hosted-v1'; deploymentId = $id
        purpose = 'ColossusLocalHelmHarness'; rootPath = $Root; primaryRoot = $Primary
        sourcePath = $Source; sourceCommit = $SourceCommit; clusterName = $Cluster; namespace = $Cluster
        port = $Port; images = @(); nodes = @(); createdAtUtc = [DateTime]::UtcNow.ToString('o')
        hosted = @{
            stage = 'planned'; ownerId = $id; projectId = 'local'; project = @{ stage = 'planned'; id = 'local' }; resources = @()
            privateAllocation = @{ path = (Get-HelmLocalAllocationPath $Cluster $id); id = $id; allocated = $false; removed = $false }
        }
    }
}

function Assert-HelmLocalFullProfile {
    param($Manifest)
    if ($null -eq $Manifest -or $Manifest.schemaVersion -ne 3 -or $Manifest.profile -cne 'full-hosted-v1') {
        throw 'Local hosting requires the complete Hosted profile. This legacy deployment is retained unchanged; use a fresh -Name or a reviewed data-preserving migration. Down and WhatIf remain available.'
    }
    if ($Manifest.deploymentId -cnotmatch '^[a-f0-9]{32}$' -or -not $Manifest.ContainsKey('hosted') -or
        $Manifest.hosted.projectId -cne 'local' -or $Manifest.hosted.ownerId -cne $Manifest.deploymentId -or -not $Manifest.hosted.ContainsKey('privateAllocation')) { throw 'Incomplete full Hosted ownership manifest.' }
    $allocation = $Manifest.hosted.privateAllocation
    $expected = Get-HelmLocalAllocationPath $Manifest.clusterName $Manifest.deploymentId
    if ($allocation.id -cne $Manifest.deploymentId -or $allocation.allocated -isnot [bool] -or $allocation.removed -isnot [bool] -or
        -not [string]::Equals([string]$allocation.path, $expected, [StringComparison]::OrdinalIgnoreCase)) { throw 'Private allocation does not match this deployment.' }
}

function Assert-HelmLocalPrivatePath {
    param([string] $Path)
    $cursor = [IO.Path]::GetFullPath($Path)
    while ($cursor) {
        if (Test-Path -LiteralPath $cursor) {
            $item = Get-Item -LiteralPath $cursor -Force
            if (-not $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Private allocation has a file or reparse ancestor.' }
        }
        $cursor = [IO.Path]::GetDirectoryName($cursor)
    }
}

function Get-HelmLocalPrivateRoot {
    param([string] $Root)
    $manifest = Get-Content -LiteralPath (Join-Path $Root 'manifest.json') -Raw | ConvertFrom-Json -AsHashtable
    if ($manifest.schemaVersion -eq 2) { return Join-Path $Root 'private' }
    Assert-HelmLocalFullProfile $manifest
    return [string]$manifest.hosted.privateAllocation.path
}

function Assert-HelmLocalPrivateAllocation {
    param([string] $Root, $Manifest)
    Assert-HelmLocalFullProfile $Manifest
    $path = [string]$Manifest.hosted.privateAllocation.path
    Assert-HelmLocalPrivatePath $path
    if (-not (Test-Path -LiteralPath $path -PathType Container)) { throw 'Recorded private material is missing. Restore the exact allocation; keys will not be regenerated.' }
    Assert-HelmLocalPrivateAcl $path
    $marker = Join-Path $path 'ownership.json'
    if (-not (Test-Path -LiteralPath $marker -PathType Leaf) -or ((Get-Item -LiteralPath $marker -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Private allocation ownership receipt is missing or linked.' }
    $owner = Get-Content -LiteralPath $marker -Raw | ConvertFrom-Json -AsHashtable
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    if ($owner.purpose -cne 'ColossusLocalHelmPrivate' -or $owner.deploymentId -cne $Manifest.deploymentId -or $owner.ownerSid -cne $sid -or
        -not [string]::Equals([string]$owner.rootPath, $Root, [StringComparison]::OrdinalIgnoreCase) -or
        -not [string]::Equals([string]$owner.privatePath, $path, [StringComparison]::OrdinalIgnoreCase)) { throw 'Private allocation belongs to another deployment.' }
    return $path
}

function Initialize-HelmLocalPrivateAllocation {
    param([string] $Root, $Manifest)
    Assert-HelmLocalFullProfile $Manifest
    $path = [string]$Manifest.hosted.privateAllocation.path
    Assert-HelmLocalPrivatePath $path
    if (Test-Path -LiteralPath $path) {
        $verified = Assert-HelmLocalPrivateAllocation $Root $Manifest
        $Manifest.hosted.privateAllocation.allocated = $true
        Write-HelmLocalJson (Join-Path $Root 'manifest.json') $Manifest
        return $verified
    }
    if ($Manifest.hosted.privateAllocation.allocated) { throw 'Previously allocated private state is missing; refusing to regenerate credentials.' }
    $parent = [IO.Path]::GetDirectoryName($path)
    Assert-HelmLocalPrivatePath $parent
    if (-not (Test-Path -LiteralPath $parent)) {
        [void][IO.Directory]::CreateDirectory($parent)
        Set-HelmLocalPrivateAcl $parent
    }
    Assert-HelmLocalPrivateAcl $parent
    [void][IO.Directory]::CreateDirectory($path)
    Set-HelmLocalPrivateAcl $path
    Write-HelmLocalJson (Join-Path $path 'ownership.json') @{
        schemaVersion = 1; purpose = 'ColossusLocalHelmPrivate'; deploymentId = $Manifest.deploymentId
        ownerSid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
        rootPath = $Root; privatePath = $path; createdAtUtc = [DateTime]::UtcNow.ToString('o')
    }
    $Manifest.hosted.privateAllocation.allocated = $true
    Write-HelmLocalJson (Join-Path $Root 'manifest.json') $Manifest
    return $path
}

function Remove-HelmLocalPrivateAllocation {
    param([string] $Root, $Manifest)
    if ($Manifest.schemaVersion -eq 2) { return }
    Assert-HelmLocalFullProfile $Manifest
    $path = [string]$Manifest.hosted.privateAllocation.path
    if ($Manifest.hosted.privateAllocation.removed -and -not (Test-Path -LiteralPath $path)) { return }
    if (-not $Manifest.hosted.privateAllocation.allocated -and -not (Test-Path -LiteralPath $path)) { return }
    $verified = Assert-HelmLocalPrivateAllocation $Root $Manifest
    Assert-HelmLocalNoChildReparse $verified
    # The exact derived, owner-bound allocation was verified; never remove its parent.
    Remove-Item -LiteralPath $verified -Recurse -Force
    $Manifest.hosted.privateAllocation.removed = $true
    Write-HelmLocalJson (Join-Path $Root 'manifest.json') $Manifest
}
