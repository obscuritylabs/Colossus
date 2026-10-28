#requires -Version 7.0
# Offline ownership/preview checks. No Docker, kind, Helm, or cluster is used.
[CmdletBinding()]
param()
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'These ACL and junction checks require Windows.' }
. (Join-Path $PSScriptRoot 'core.ps1')
$entrypoint = Join-Path $PSScriptRoot '../helm-local.ps1'
$source = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$git = Resolve-HelmLocalTool 'git' ''
$common = (Invoke-HelmLocalNative $git @('-C', $source, 'rev-parse', '--path-format=absolute', '--git-common-dir')).Trim()
$primary = [IO.Path]::GetDirectoryName($common.TrimEnd('\', '/'))
$fixture = Join-Path $primary '.local/tests/helm-harness-contract'
$cluster = 'colossus-harness-contract'
if (Test-Path -LiteralPath $fixture) { throw 'The contract fixture already exists; inspect it before running this test.' }

function Assert-Fails {
    param([scriptblock] $Operation, [string] $Message)
    $failed = $false
    try { & $Operation | Out-Null } catch { $failed = $true }
    if (-not $failed) { throw $Message }
}

try {
    & $entrypoint -Action down -Name harness-contract -WhatIf | Out-Null
    if (Test-Path -LiteralPath $fixture) { throw 'An absent-directory preview created state.' }
    [void][IO.Directory]::CreateDirectory($fixture)
    Set-HelmLocalPrivateAcl $fixture
    $manifest = @{
        schemaVersion = 2; purpose = 'ColossusLocalHelmHarness'; rootPath = $fixture; primaryRoot = $primary
        sourcePath = $source; sourceCommit = ('a' * 40); clusterName = $cluster; namespace = $cluster
        port = 8443; images = @(); nodes = @(@{ id = 'exact-owned-container'; name = 'owned-control-plane' })
    }
    $path = Join-Path $fixture 'manifest.json'
    Write-HelmLocalJson $path $manifest
    $hash = (Get-FileHash -LiteralPath $path).Hash
    $before = Save-HelmLocalEnvironment
    # Invalid executable paths prove the preview never resolves or invokes clients.
    & $entrypoint -Action down -Name harness-contract -RemoveFiles -RemoveImages -WhatIf -Docker 'Z:/not-an-executable.exe' -Kind 'Z:/not-an-executable.exe' | Out-Null
    if ((Get-FileHash -LiteralPath $path).Hash -cne $hash) { throw 'Preview changed ownership state.' }
    if (@(Get-ChildItem -LiteralPath $fixture -Force).Count -ne 1) { throw 'Preview created files.' }
    $after = Save-HelmLocalEnvironment
    foreach ($key in $before.Keys) { if ($before[$key] -cne $after[$key]) { throw "Preview changed environment: $key" } }

    [void][IO.Directory]::CreateDirectory((Join-Path $fixture 'private'))
    New-HelmLocalBrowserCertificate $fixture
    $certificate = [Security.Cryptography.X509Certificates.X509Certificate2]::CreateFromPemFile((Join-Path $fixture 'private/browser-cert.pem'), (Join-Path $fixture 'private/browser-key.pem'))
    try {
        if ($certificate.GetNameInfo('DnsName', $false) -cne 'colossus.localhost' -or -not $certificate.HasPrivateKey) { throw 'Browser certificate identity or private key is incorrect.' }
    } finally { $certificate.Dispose() }
    $image = Convert-HelmLocalImage 'colossus-local-control-plane:sample' 'Never'
    if ($image.registry -cne 'docker.io' -or $image.repository -cne 'library/colossus-local-control-plane' -or $image.tag -cne 'sample' -or $image.pullPolicy -cne 'Never') { throw 'Explicit local image reference changed.' }
    $image = Convert-HelmLocalImage 'ghcr.io/obscuritylabs/colossus-control-plane:v0.11.11-preview.4' 'IfNotPresent'
    if ($image.registry -cne 'ghcr.io' -or $image.repository -cne 'obscuritylabs/colossus-control-plane') { throw 'Published image registry changed.' }

    $manifest.clusterName = 'colossus-someone-else'
    Write-HelmLocalJson $path $manifest
    Assert-Fails { & $entrypoint -Action down -Name harness-contract -WhatIf } 'A mismatched cluster manifest was accepted.'
    $manifest.clusterName = $cluster
    Write-HelmLocalJson $path $manifest
    Assert-Fails { Assert-HelmLocalBoundary (Join-Path $primary '.local/not-tests/helm-harness-contract') $primary } 'An outside path was accepted.'
    Assert-Fails { Assert-HelmLocalNodes @(@{ id = 'reused-name-other-container' }) $manifest.nodes } 'A replacement cluster container was accepted.'

    $target = Join-Path $fixture 'target'
    $junction = Join-Path $fixture 'junction'
    [void][IO.Directory]::CreateDirectory($target)
    $null = New-Item -ItemType Junction -Path $junction -Target $target
    Assert-Fails { & $entrypoint -Action down -Name harness-contract -WhatIf } 'A child junction was accepted.'
    Remove-Item -LiteralPath $junction -Force
    Assert-HelmLocalNoChildReparse $fixture

    # Simulate an A -> B upgrade without Docker. A remains owned even though B is
    # selected now; older unrelated images must never acquire cleanup authority.
    $oldImage = @{ reference = 'colossus-test:a'; id = 'sha256:old-owned'; owned = $true }
    $newImage = @{ reference = 'colossus-test:b'; id = 'sha256:new-owned'; owned = $true }
    $unowned = @{ reference = 'someone-else:shared'; id = 'sha256:unowned'; owned = $false }
    $merged = @(Merge-HelmLocalImageReceipts @($oldImage, $unowned) @($newImage, $unowned))
    $merged = @(Merge-HelmLocalImageReceipts $merged @($newImage, $unowned))
    if ($merged.Count -ne 3 -or @($merged | Where-Object owned).Count -ne 2) { throw 'Upgrade lost or duplicated image ownership receipts.' }
    $moved = @(Merge-HelmLocalImageReceipts @($oldImage) @(@{ reference = $oldImage.reference; id = 'sha256:moved'; owned = $true }))
    if ($moved.Count -ne 2) { throw 'Moving a tag discarded its earlier exact-ID receipt.' }

    $savedNative = (Get-Item Function:Invoke-HelmLocalNative).ScriptBlock
    $savedNodes = (Get-Item Function:Get-HelmLocalNodes).ScriptBlock
    $savedImage = (Get-Item Function:Get-HelmLocalImageReceipt).ScriptBlock
    try {
        $script:removedImagesForTest = [Collections.Generic.List[string]]::new()
        $script:imagesForTest = @{ $oldImage.reference = $oldImage; $newImage.reference = $newImage }
        $script:imageIdsForTest = ''
        function Get-HelmLocalNodes { param($Tools, $Cluster) return @() }
        function Get-HelmLocalImageReceipt { param($Tools, $Reference, $Root) return $script:imagesForTest[$Reference] }
        function Invoke-HelmLocalNative {
            param($Executable, $Arguments)
            if ($Executable -ceq 'offline-docker' -and $Arguments[0] -ceq 'image' -and $Arguments[1] -ceq 'ls') { return $script:imageIdsForTest }
            if ($Executable -cne 'offline-docker' -or $Arguments[0] -cne 'image' -or $Arguments[1] -cne 'rm') { throw 'Unexpected native operation in offline cleanup test.' }
            $script:removedImagesForTest.Add($Arguments[2])
            return ''
        }
        $manifest.images = $merged
        $manifest.nodes = @()
        Write-HelmLocalJson $path $manifest
        $receiptHash = (Get-FileHash -LiteralPath $path).Hash
        $script:imagesForTest[$oldImage.reference] = @{ reference = $oldImage.reference; id = 'sha256:moved'; owned = $true }
        Assert-Fails { Remove-HelmLocalDeployment @{ Docker = 'offline-docker'; Kind = 'offline-kind' } $manifest $fixture $primary -RemoveImages -RemoveFiles } 'Cleanup accepted a moved tag.'
        if ($script:removedImagesForTest.Count -or (Get-FileHash -LiteralPath $path).Hash -cne $receiptHash) { throw 'Moved-tag refusal changed resources or discarded the receipt.' }
        $script:imagesForTest[$oldImage.reference] = $oldImage
        Remove-HelmLocalDeployment @{ Docker = 'offline-docker'; Kind = 'offline-kind' } $manifest $fixture $primary -RemoveImages | Out-Null
        if (($script:removedImagesForTest -join ',') -cne 'colossus-test:a,colossus-test:b') { throw 'Upgrade cleanup failed to include both owned generations or selected an unrelated image.' }
        $script:removedImagesForTest.Clear()
        $script:imagesForTest[$oldImage.reference] = @{ reference = $oldImage.reference; id = ''; owned = $false }
        $script:imageIdsForTest = $oldImage.id
        Assert-Fails { Remove-HelmLocalDeployment @{ Docker = 'offline-docker'; Kind = 'offline-kind' } $manifest $fixture $primary -RemoveImages -RemoveFiles } 'Cleanup discarded an untagged image receipt while its recorded ID still existed.'
        if ($script:removedImagesForTest.Count -or (Get-FileHash -LiteralPath $path).Hash -cne $receiptHash) { throw 'Untagged-image refusal changed resources or discarded its receipt.' }
        $script:imageIdsForTest = ''
        Remove-HelmLocalDeployment @{ Docker = 'offline-docker'; Kind = 'offline-kind' } $manifest $fixture $primary -RemoveImages | Out-Null
        if (($script:removedImagesForTest -join ',') -cne 'colossus-test:b') { throw 'A fully removed image prevented safe cleanup retry.' }
    } finally {
        Set-Item Function:Invoke-HelmLocalNative $savedNative
        Set-Item Function:Get-HelmLocalNodes $savedNodes
        Set-Item Function:Get-HelmLocalImageReceipt $savedImage
        Remove-Variable -Scope Script removedImagesForTest, imagesForTest, imageIdsForTest -ErrorAction SilentlyContinue
    }
    Write-Output 'PASS: offline preview, ACL, environment, ownership, container identity, certificate/key, image references, boundary/reparse refusal, cumulative upgrade cleanup and moved-tag receipt retention.'
} finally {
    # Remove only this exact synthetic fixture, never live deployment resources.
    if (Test-Path -LiteralPath $fixture) {
        $junction = Join-Path $fixture 'junction'
        if (Test-Path -LiteralPath $junction) {
            $item = Get-Item -LiteralPath $junction -Force
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { Remove-Item -LiteralPath $junction -Force }
        }
        Assert-HelmLocalBoundary $fixture $primary
        Assert-HelmLocalNoChildReparse $fixture
        Remove-Item -LiteralPath $fixture -Recurse -Force
    }
}
