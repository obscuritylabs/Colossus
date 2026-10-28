#requires -Version 7.2
# Offline profile/PKI/ownership checks. No Docker, kind, Helm, Kubernetes, or provider is used.
[CmdletBinding()]
param()
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'These ACL and junction checks require Windows.' }
. (Join-Path $PSScriptRoot 'core.ps1')
. (Join-Path $PSScriptRoot 'private.ps1')
. (Join-Path $PSScriptRoot 'hosted.ps1')
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

$allocatedManifestForCleanup = $null
function Get-ContractFileHashes {
    param([string] $Root)
    Assert-HelmLocalNoChildReparse $Root
    $files = @(Get-ChildItem -LiteralPath $Root -Recurse -Force | Sort-Object FullName | ForEach-Object {
        @{ name = [IO.Path]::GetRelativePath($Root, $_.FullName); directory = $_.PSIsContainer; sha256 = $(if ($_.PSIsContainer) { '' } else { (Get-FileHash -LiteralPath $_.FullName).Hash }) }
    })
    return ($files | ConvertTo-Json -Depth 4 -Compress)
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

    foreach ($action in @('up', 'test')) {
        $legacyRejected = $false
        try {
            & $entrypoint -Action $action -Name harness-contract -Port 8443 -Docker 'Z:/not-an-executable.exe' -Kind 'Z:/not-an-executable.exe' -Helm 'Z:/not-an-executable.exe' -Kubectl 'Z:/not-an-executable.exe' -Node 'Z:/not-an-executable.exe' | Out-Null
        } catch {
            $legacyRejected = $_.Exception.Message -match '(?i)(legacy|older|profile|migration|fresh.*name)'
        }
        if (-not $legacyRejected -or (Get-FileHash -LiteralPath $path).Hash -cne $hash -or @(Get-ChildItem -LiteralPath $fixture -Force).Count -ne 1) { throw 'Legacy up/test did not fail before mutating files or resolving deployment clients.' }
        $after = Save-HelmLocalEnvironment
        foreach ($key in $before.Keys) { if ($before[$key] -cne $after[$key]) { throw "Legacy refusal changed environment: $key" } }
    }

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
    $selectedOnly = @(Merge-HelmLocalImageReceipts -Previous $null -Selected @($newImage, $null, $unowned))
    if ($selectedOnly.Count -ne 2 -or @($selectedOnly | Where-Object { $null -eq $_ }).Count -or
        @($selectedOnly | Where-Object reference -CEQ $newImage.reference).Count -ne 1) { throw 'A fresh deployment lost image receipts when previous history was null.' }
    $previousOnly = @(Merge-HelmLocalImageReceipts -Previous @($oldImage, $null, $unowned) -Selected $null)
    if ($previousOnly.Count -ne 1 -or $previousOnly[0].id -cne $oldImage.id) { throw 'Null selected images lost owned history or adopted an unrelated image.' }
    $emptyImages = @(Merge-HelmLocalImageReceipts -Previous $null -Selected $null)
    if ($emptyImages.Count) { throw 'Empty image inputs produced a null receipt.' }

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
    # A failed atomic promotion must keep the previous complete receipt. A
    # competing operation must be excluded by the OS handle, not a stale PID flag.
    $atomicPath = Join-Path $fixture 'atomic-contract.json'
    Write-HelmLocalJson $atomicPath @{ generation = 1; payload = ('x' * 32768) }
    $atomicHash = (Get-FileHash -LiteralPath $atomicPath).Hash
    $reader = [IO.FileStream]::new($atomicPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        Assert-Fails { Write-HelmLocalJson $atomicPath @{ generation = 2; payload = ('y' * 32768) } } 'A receipt was replaced despite an exclusive promotion conflict.'
        if ((Get-FileHash -LiteralPath $atomicPath).Hash -cne $atomicHash) { throw 'Failed atomic promotion changed the previous complete receipt.' }
    } finally { $reader.Dispose() }
    foreach ($generation in 2..4) {
        Write-HelmLocalJson $atomicPath @{ generation = $generation; payload = ('z' * 32768) }
        $actual = Get-Content -LiteralPath $atomicPath -Raw | ConvertFrom-Json -AsHashtable
        if ($actual.generation -ne $generation -or $actual.payload.Length -ne 32768) { throw 'Repeated atomic writes exposed an incomplete receipt.' }
    }
    $operation = Enter-HelmLocalRunLock -Primary $primary -Cluster $cluster
    $parallel = $null
    try {
        Assert-Fails { $unexpected = Enter-HelmLocalRunLock -Primary $primary -Cluster $cluster; $unexpected.Dispose() } 'A second operation acquired the same deployment lock.'
        $parallel = Enter-HelmLocalRunLock -Primary $primary -Cluster 'colossus-harness-contract-other'
    } finally { if ($null -ne $parallel) { $parallel.Dispose() }; $operation.Dispose() }
    $operation = Enter-HelmLocalRunLock -Primary $primary -Cluster $cluster
    $operation.Dispose()

    foreach ($database in @(
        @{ name = 'colossus-controller-postgres'; request = '512Mi'; limit = '1Gi'; size = '8Gi'; user = 'colossus'; database = 'colossus' },
        @{ name = 'colossus-anchor-postgres'; request = '256Mi'; limit = '512Mi'; size = '1Gi'; user = 'anchor_app'; database = 'colossus_anchor' }
    )) {
        $objects = @(New-HelmLocalDatabaseObjects -Namespace "$cluster-data" -Name $database.name -Database $database.database -User $database.user -Size $database.size -Image 'postgres:synthetic-only')
        $stateful = @($objects | Where-Object kind -CEQ 'StatefulSet')
        $claim = @($objects | Where-Object kind -CEQ 'PersistentVolumeClaim')
        if ($stateful.Count -ne 1 -or $claim.Count -ne 1) { throw 'Generated database resources are incomplete.' }
        $resources = $stateful[0].spec.template.spec.containers[0].resources
        if ($resources.requests.memory -cne $database.request -or $resources.limits.memory -cne $database.limit -or
            $claim[0].spec.resources.requests.storage -cne $database.size) { throw 'Generated local database budgets changed or crossed authority domains.' }
    }

    # The current default is complete Hosted. Legacy receipts remain sufficient for
    # deliberate cleanup, but cannot select an older database/profile during up/test.
    $full = New-HelmLocalManifest -Root $fixture -Primary $primary -Source $source -Cluster $cluster -Port 8443 -SourceCommit ('a' * 40)
    $allocatedManifestForCleanup = $full
    if ($full.schemaVersion -ne 3 -or $full.profile -cne 'full-hosted-v1' -or $full.hosted.projectId -cne 'local' -or
        $full.hosted.stage -cne 'planned' -or $full.hosted.privateAllocation.allocated) { throw 'A new local deployment did not select the complete Hosted profile.' }
    Assert-HelmLocalFullProfile -Manifest $full
    $deploymentId = $full.deploymentId
    $plannedPrivate = [string]$full.hosted.privateAllocation.path
    if ([IO.Path]::GetFileName($plannedPrivate) -cne "helm-harness-contract-$deploymentId") { throw 'Private allocation lost its deployment name or ID.' }
    if (Test-Path -LiteralPath $plannedPrivate) { throw 'Planning unexpectedly allocated private state.' }
    $invalid = $full | ConvertTo-Json -Depth 30 | ConvertFrom-Json -AsHashtable
    $invalid.profile = 'fleet-only'
    Assert-Fails { Assert-HelmLocalFullProfile -Manifest $invalid } 'A Fleet-only fallback profile was accepted.'
    $invalid = $full | ConvertTo-Json -Depth 30 | ConvertFrom-Json -AsHashtable
    $invalid.hosted.privateAllocation.path = Join-Path $fixture 'private'
    Assert-Fails { Assert-HelmLocalFullProfile -Manifest $invalid } 'Private custody could be redirected into the workspace.'
    Write-HelmLocalJson $path $full
    $plannedHash = (Get-FileHash -LiteralPath $path).Hash
    $plannedFiles = Get-ContractFileHashes $fixture
    $before = Save-HelmLocalEnvironment
    & $entrypoint -Action up -Name harness-contract -Port 8443 -WhatIf -Docker 'Z:/not-an-executable.exe' -Kind 'Z:/not-an-executable.exe' -Helm 'Z:/not-an-executable.exe' -Kubectl 'Z:/not-an-executable.exe' -Node 'Z:/not-an-executable.exe' | Out-Null
    & $entrypoint -Action down -Name harness-contract -RemoveFiles -WhatIf -Docker 'Z:/not-an-executable.exe' -Kind 'Z:/not-an-executable.exe' | Out-Null
    if ((Test-Path -LiteralPath $plannedPrivate) -or (Get-FileHash -LiteralPath $path).Hash -cne $plannedHash -or
        (Get-ContractFileHashes $fixture) -cne $plannedFiles) { throw 'A preview created or modified private material or receipts.' }
    $after = Save-HelmLocalEnvironment
    foreach ($key in $before.Keys) { if ($before[$key] -cne $after[$key]) { throw "Profile preview changed environment: $key" } }

    $allocation = Initialize-HelmLocalPrivateAllocation -Root $fixture -Manifest $full
    if ($allocation -isnot [string] -or $allocation -cne $plannedPrivate -or (Get-HelmLocalPrivateRoot -Root $fixture) -cne $allocation) { throw 'The private allocator returned unexpected output or changed identity.' }
    $ownerPath = Join-Path $allocation 'ownership.json'
    $ownerHash = (Get-FileHash -LiteralPath $ownerPath).Hash
    $again = Initialize-HelmLocalPrivateAllocation -Root $fixture -Manifest $full
    if ($again -cne $allocation -or $full.deploymentId -cne $deploymentId -or (Get-FileHash -LiteralPath $ownerPath).Hash -cne $ownerHash) { throw 'Reusing the profile changed private allocation identity.' }
    $ownerBytes = [IO.File]::ReadAllBytes($ownerPath)
    try {
        $wrongOwner = Get-Content -LiteralPath $ownerPath -Raw | ConvertFrom-Json -AsHashtable
        $wrongOwner.deploymentId = ('b' * 32)
        Write-HelmLocalJson $ownerPath $wrongOwner
        Assert-Fails { Assert-HelmLocalPrivateAllocation -Root $fixture -Manifest $full } 'An allocation belonging to another deployment was accepted.'
    } finally { [IO.File]::WriteAllBytes($ownerPath, $ownerBytes) }
    $privateJunction = Join-Path $allocation 'linked-child'
    $null = New-Item -ItemType Junction -Path $privateJunction -Target (Join-Path $fixture 'target')
    try {
        Assert-Fails { Remove-HelmLocalPrivateAllocation -Root $fixture -Manifest $full } 'Cleanup traversed a private allocation junction.'
        if (-not (Test-Path -LiteralPath $allocation)) { throw 'Rejected private cleanup removed the allocation.' }
    } finally {
        if ((Get-Item -LiteralPath $privateJunction -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { Remove-Item -LiteralPath $privateJunction -Force }
    }

    # These are synthetic local keys only. Compare bytes without releasing them to
    # logs; repeated provisioning must preserve every retained master and CA.
    $full.hosted.ownerId = $deploymentId
    $material = Initialize-HelmLocalHostedMaterial $fixture $cluster $full
    if ($material -isnot [string]) { throw 'Material initialization leaked extra pipeline output.' }
    $materialHashes = Get-ContractFileHashes $material
    $materialReceipt = $full.hosted.material.receiptSha256
    $again = Initialize-HelmLocalHostedMaterial $fixture $cluster $full
    if ($again -cne $material -or (Get-ContractFileHashes $material) -cne $materialHashes -or $full.hosted.material.receiptSha256 -cne $materialReceipt) { throw 'Repeated provisioning rotated retained Hosted material.' }
    $caHashes = @('controller', 'anchorDatabase', 'anchorHttps' | ForEach-Object { (Get-FileHash -LiteralPath (Join-Path $material "$_-ca.pem")).Hash } | Sort-Object -Unique)
    if ($caHashes.Count -ne 3) { throw 'Independent trust domains shared a CA.' }
    $masterValues = @('journal-key', 'signing-key', 'wrapping-key', 'anchor-storage-key' | ForEach-Object { [IO.File]::ReadAllText((Join-Path $material $_)) })
    if (@($masterValues | Sort-Object -Unique).Count -ne 4 -or @($masterValues | Where-Object { $_ -cnotmatch '^[0-9a-f]{64}$' }).Count) { throw 'Independent master material is missing or malformed.' }
    foreach ($missing in @('journal-key', 'material.json')) {
        $leaf = Join-Path $material $missing
        $bytes = [IO.File]::ReadAllBytes($leaf)
        try {
            [IO.File]::Delete($leaf)
            Assert-Fails { Initialize-HelmLocalHostedMaterial $fixture $cluster $full } 'Missing retained Hosted material was regenerated.'
            if (Test-Path -LiteralPath $leaf) { throw 'A missing retained key or receipt was recreated.' }
        } finally { [IO.File]::WriteAllBytes($leaf, $bytes) }
    }
    $receiptPath = Join-Path $material 'material.json'
    $receiptBytes = [IO.File]::ReadAllBytes($receiptPath)
    try {
        [IO.File]::WriteAllText($receiptPath, '{}')
        Assert-Fails { Initialize-HelmLocalHostedMaterial $fixture $cluster $full } 'A changed retained material receipt was accepted.'
        if ($full.hosted.material.receiptSha256 -cne $materialReceipt) { throw 'A changed receipt replaced the retained material pin.' }
    } finally { [IO.File]::WriteAllBytes($receiptPath, $receiptBytes) }
    # Remove only these verified synthetic leaves, then a now-empty directory.
    # This proves directory disappearance also fails closed, without a C: rename
    # or any recursive deletion outside the production ownership-checked helper.
    $savedMaterial = @{}
    foreach ($file in Get-ChildItem -LiteralPath $material -File) { $savedMaterial[$file.Name] = [IO.File]::ReadAllBytes($file.FullName) }
    Assert-HelmLocalNoChildReparse $allocation
    if ([IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($material)) -cne $allocation -or [IO.Path]::GetFileName($material) -cne 'hosted-material') { throw 'Synthetic material cleanup boundary changed.' }
    try {
        foreach ($name in $savedMaterial.Keys) { [IO.File]::Delete((Join-Path $material $name)) }
        [IO.Directory]::Delete($material, $false)
        Assert-Fails { Initialize-HelmLocalHostedMaterial $fixture $cluster $full } 'A missing retained material directory regenerated keys.'
        if (Test-Path -LiteralPath $material) { throw 'Missing retained material directory was recreated.' }
    } finally {
        [void][IO.Directory]::CreateDirectory($material)
        foreach ($name in $savedMaterial.Keys) { [IO.File]::WriteAllBytes((Join-Path $material $name), $savedMaterial[$name]) }
    }
    if ((Get-ContractFileHashes $material) -cne $materialHashes) { throw 'A failed material check changed another retained file.' }

    # Exercise production create/reconcile behavior with a harmless ConfigMap and
    # an in-memory Kubernetes boundary; no cluster or credentials are involved.
    $savedKubectl = (Get-Item Function:Invoke-HelmLocalKubectl).ScriptBlock
    try {
        $script:contractObjects = @{}
        $script:contractCreates = 0
        $script:contractImagePatches = 0
        $script:contractLosePatchResponse = $false
        $script:contractLoseCreateResponse = $false
        function Invoke-HelmLocalKubectl {
            param($Tools, [string] $Root, [string] $Cluster, [string[]] $Arguments)
            if ($Cluster -cne 'colossus-harness-contract' -or $Root -cne $fixture -or $Arguments.Count -lt 3) { throw 'Offline Kubernetes call omitted its exact deployment context or arguments.' }
            if ($Arguments[0] -ceq 'get') {
                $key = "$($Arguments[1])/$($Arguments[2])"
                if ($script:contractObjects.ContainsKey($key)) { return ($script:contractObjects[$key] | ConvertTo-Json -Depth 30 -Compress) }
                return ''
            }
            $patchIndex = [Array]::IndexOf($Arguments, 'patch')
            if ($patchIndex -ge 0) {
                if ($Arguments[$patchIndex + 1] -cne 'deployment') { throw 'Unexpected offline patch target.' }
                $key = "Deployment/$($Arguments[$patchIndex + 2])"
                $existing = $script:contractObjects[$key]
                $fileIndex = [Array]::IndexOf($Arguments, '--patch-file')
                if ($fileIndex -lt 0) { throw 'Image patch has no reviewed patch file.' }
                $operations = @(Get-Content -LiteralPath $Arguments[$fileIndex + 1] -Raw | ConvertFrom-Json -AsHashtable)
                $tests = @{}
                $nextImage = $null
                foreach ($operation in $operations) {
                    $actual = switch ($operation.path) {
                        '/metadata/uid' { $existing.metadata.uid }
                        '/metadata/resourceVersion' { $existing.metadata.resourceVersion }
                        '/spec/template/spec/containers/0/image' { $existing.spec.template.spec.containers[0].image }
                        default { throw 'Patch crossed the image-only upgrade boundary.' }
                    }
                    if ($operation.op -ceq 'test') {
                        if ($actual -cne $operation.value) { throw 'Image patch precondition failed.' }
                        $tests[$operation.path] = $true
                    } elseif ($operation.op -ceq 'replace' -and $operation.path -ceq '/spec/template/spec/containers/0/image') { $nextImage = $operation.value }
                    else { throw 'Patch changed more than the application image.' }
                }
                if ($tests.Count -ne 3 -or -not $nextImage) { throw 'Image patch omitted UID, resourceVersion or old-image authority.' }
                $object = $existing | ConvertTo-Json -Depth 30 | ConvertFrom-Json -AsHashtable
                $object.spec.template.spec.containers[0].image = $nextImage
                $object.metadata.resourceVersion = [string]([int]$existing.metadata.resourceVersion + 1)
                $script:contractImagePatches++
                $script:contractObjects[$key] = $object
                if ($script:contractLosePatchResponse) { $script:contractLosePatchResponse = $false; throw 'Synthetic lost image-patch response.' }
                return ($object | ConvertTo-Json -Depth 30 -Compress)
            }
            if ($Arguments[0] -cne 'create' -or $Arguments[1] -cne '-f') { throw 'Unexpected offline Kubernetes operation.' }
            $object = Get-Content -LiteralPath $Arguments[2] -Raw | ConvertFrom-Json -AsHashtable
            $key = "$($object.kind)/$($object.metadata.name)"
            if ($script:contractObjects.ContainsKey($key)) { throw 'Create attempted to replace an existing object.' }
            $script:contractCreates++
            $object.metadata.uid = "synthetic-object-$($script:contractCreates)"
            $object.metadata.resourceVersion = '1'
            $script:contractObjects[$key] = $object
            if ($script:contractLoseCreateResponse) { $script:contractLoseCreateResponse = $false; throw 'Synthetic lost create response.' }
            return ($object | ConvertTo-Json -Depth 30 -Compress)
        }
        $desired = @{ apiVersion = 'v1'; kind = 'ConfigMap'; metadata = @{ name = 'contract-state'; namespace = $cluster }; data = @{ fixture = 'synthetic-only' } }
        Ensure-HelmLocalHostedObject @{} $fixture $full $desired
        Ensure-HelmLocalHostedObject @{} $fixture $full $desired
        if ($script:contractCreates -ne 1 -or $full.hosted.resources.Count -ne 1 -or $full.hosted.resources[0].uid -cne 'synthetic-object-1') { throw 'Repeated provisioning replaced or duplicated retained ownership.' }
        $ownedObject = $script:contractObjects['ConfigMap/contract-state']
        $ownedObject.metadata.uid = 'replacement-object'
        Assert-Fails { Ensure-HelmLocalHostedObject @{} $fixture $full $desired } 'A replaced resource UID was adopted.'
        $ownedObject.metadata.uid = 'synthetic-object-1'
        [void]$script:contractObjects.Remove('ConfigMap/contract-state')
        Assert-Fails { Ensure-HelmLocalHostedObject @{} $fixture $full $desired } 'A missing retained object was recreated.'
        $script:contractObjects['ConfigMap/contract-state'] = $ownedObject
        $script:contractLoseCreateResponse = $true
        $recoverable = @{ apiVersion = 'v1'; kind = 'ConfigMap'; metadata = @{ name = 'contract-response-lost'; namespace = $cluster }; data = @{ fixture = 'synthetic-only' } }
        Assert-Fails { Ensure-HelmLocalHostedObject @{} $fixture $full $recoverable } 'Lost create response did not stop the first attempt.'
        Ensure-HelmLocalHostedObject @{} $fixture $full $recoverable
        if ($script:contractCreates -ne 2 -or $full.hosted.resources.Count -ne 2 -or @($full.hosted.resources | Where-Object { -not $_.uid }).Count) { throw 'Owned create-response recovery issued another create or lost its receipt.' }
        $anchor = @{ apiVersion = 'apps/v1'; kind = 'Deployment'; metadata = @{ name = 'colossus-chat-anchor'; namespace = "$cluster-anchors" }; spec = @{ template = @{ spec = @{ containers = @(@{ name = 'anchor'; image = 'synthetic-anchor:a' }) } } } }
        Ensure-HelmLocalHostedObject @{} $fixture $full $anchor
        $anchorUid = $script:contractObjects['Deployment/colossus-chat-anchor'].metadata.uid
        $nextAnchor = $anchor | ConvertTo-Json -Depth 30 | ConvertFrom-Json -AsHashtable
        $nextAnchor.spec.template.spec.containers[0].image = 'synthetic-anchor:b'
        $script:contractLosePatchResponse = $true
        Assert-Fails { Ensure-HelmLocalHostedObject @{} $fixture $full $nextAnchor } 'Lost image-patch response did not stop the first attempt.'
        Ensure-HelmLocalHostedObject @{} $fixture $full $nextAnchor
        $actualAnchor = $script:contractObjects['Deployment/colossus-chat-anchor']
        if ($script:contractCreates -ne 3 -or $script:contractImagePatches -ne 1 -or $actualAnchor.metadata.uid -cne $anchorUid -or $actualAnchor.spec.template.spec.containers[0].image -cne 'synthetic-anchor:b') { throw 'Image-upgrade recovery repeated its patch or changed resource ownership.' }
        $badAnchor = $nextAnchor | ConvertTo-Json -Depth 30 | ConvertFrom-Json -AsHashtable
        $badAnchor.spec.template.spec.containers[0].name = 'unreviewed-authority-change'
        Assert-Fails { Ensure-HelmLocalHostedObject @{} $fixture $full $badAnchor } 'Image upgrade implicitly authorized another infrastructure change.'
        if ($script:contractImagePatches -ne 1) { throw 'Rejected infrastructure drift mutated the owned object.' }

    } finally {
        Set-Item Function:Invoke-HelmLocalKubectl $savedKubectl
        Remove-Variable -Scope Script contractObjects, contractCreates, contractLoseCreateResponse, contractImagePatches, contractLosePatchResponse -ErrorAction SilentlyContinue
    }

    $savedKubectl = (Get-Item Function:Invoke-HelmLocalKubectl).ScriptBlock
    try {
        $script:contractChartSecrets = @{}
        foreach ($name in @('colossus-control-plane-state', 'colossus-control-plane-tls')) {
            $script:contractChartSecrets[$name] = @{
                apiVersion = 'v1'; kind = 'Secret'
                metadata = @{ name = $name; uid = "synthetic-$name"; annotations = @{ 'meta.helm.sh/release-name' = 'colossus-control-plane'; 'meta.helm.sh/release-namespace' = $cluster } }
                data = @{ marker = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes("synthetic-$name")) }
            }
        }
        function Invoke-HelmLocalKubectl {
            param($Tools, [string] $Root, [string] $Cluster, [string[]] $Arguments)
            if ($Cluster -cne 'colossus-harness-contract' -or $Root -cne $fixture -or $Arguments[0] -cne '-n' -or $Arguments[1] -cne $Cluster -or $Arguments[2] -cne 'get' -or $Arguments[3] -cne 'secret') { throw 'Chart authority contract attempted a non-read operation or wrong context.' }
            if ($script:contractChartSecrets.ContainsKey($Arguments[4])) { return ($script:contractChartSecrets[$Arguments[4]] | ConvertTo-Json -Depth 20 -Compress) }
            return ''
        }
        Sync-HelmLocalChartSecrets @{} $fixture $full -FirstInstall
        Assert-HelmLocalChartSecretsComplete $full
        $authorityFiles = Get-ContractFileHashes $allocation
        $authorityReceipt = Get-Content -LiteralPath $path -Raw
        Sync-HelmLocalChartSecrets @{} $fixture $full
        if ($full.hosted.chartSecrets.Count -ne 2 -or (Get-ContractFileHashes $allocation) -cne $authorityFiles -or (Get-Content -LiteralPath $path -Raw) -cne $authorityReceipt) { throw 'Repeated authority checks changed retained chart keys or receipts.' }
        $secret = $script:contractChartSecrets['colossus-control-plane-state']
        $data = $secret.data.marker
        try {
            $secret.data.marker = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes('unexpected-rotation'))
            Assert-Fails { Sync-HelmLocalChartSecrets @{} $fixture $full } 'Changed chart authority data was accepted.'
        } finally { $secret.data.marker = $data }
        $uid = $secret.metadata.uid
        try {
            $secret.metadata.uid = 'replacement-secret'
            Assert-Fails { Sync-HelmLocalChartSecrets @{} $fixture $full } 'Replacement chart Secret UID was adopted.'
        } finally { $secret.metadata.uid = $uid }
        [void]$script:contractChartSecrets.Remove('colossus-control-plane-state')
        try { Assert-Fails { Sync-HelmLocalChartSecrets @{} $fixture $full } 'Missing chart authority was silently replaced.' }
        finally { $script:contractChartSecrets['colossus-control-plane-state'] = $secret }
        $backup = [string]$full.hosted.chartSecrets[0].privateBackup
        $bytes = [IO.File]::ReadAllBytes($backup)
        try {
            [IO.File]::Delete($backup)
            Assert-Fails { Sync-HelmLocalChartSecrets @{} $fixture $full } 'Missing retained authority backup was recreated.'
            if (Test-Path -LiteralPath $backup) { throw 'Authority check regenerated a missing backup.' }
        } finally { [IO.File]::WriteAllBytes($backup, $bytes) }
        if ((Get-ContractFileHashes $allocation) -cne $authorityFiles -or (Get-Content -LiteralPath $path -Raw) -cne $authorityReceipt) { throw 'Rejected authority changes mutated another key or receipt.' }
    } finally {
        Set-Item Function:Invoke-HelmLocalKubectl $savedKubectl
        Remove-Variable -Scope Script contractChartSecrets -ErrorAction SilentlyContinue
    }

    # Only a verified cluster deletion permits new resource UIDs. Retired receipts
    # remain cumulative; private CA/master material never becomes disposable.
    $full.hosted.volumes = @{ synthetic = @{ pvUid = 'synthetic-volume'; pvcUid = 'synthetic-claim' } }
    $full.clusterRemoved = $false
    $beforeGeneration = Get-HelmLocalCanonicalJson $full
    Reset-HelmLocalRemovedGeneration $fixture $full
    if ((Get-HelmLocalCanonicalJson $full) -cne $beforeGeneration) { throw 'A live generation was reset without cluster deletion.' }
    $oldResources = @($full.hosted.resources).Count
    $full.clusterRemoved = $true
    Reset-HelmLocalRemovedGeneration $fixture $full
    if ($full.hosted.resources.Count -or $full.hosted.chartSecrets.Count -or $full.hosted.volumes -isnot [Collections.IDictionary] -or $full.hosted.volumes.Count -or $full.hosted.retiredGenerations.Count -ne 1 -or
        $full.hosted.retiredGenerations[0].resources.Count -ne $oldResources -or $full.hosted.retiredGenerations[0].chartSecrets.Count -ne 2 -or $full.hosted.retiredGenerations[0].volumes.synthetic.pvUid -cne 'synthetic-volume' -or
        $full.hosted.project.stage -cne 'planned' -or $full.deploymentId -cne $deploymentId -or
        (Get-ContractFileHashes $material) -cne $materialHashes) { throw 'Removed-generation reset lost ownership history or rotated retained private material.' }
    Reset-HelmLocalRemovedGeneration $fixture $full
    if ($full.hosted.retiredGenerations.Count -ne 1) { throw 'Repeated reset duplicated ownership generations.' }

    $savedNodes = (Get-Item Function:Get-HelmLocalNodes).ScriptBlock
    $cleanupLock = Enter-HelmLocalRunLock -Primary $primary -Cluster $cluster
    try {
        function Get-HelmLocalNodes { param($Tools, $Cluster) return @() }
        $allocatedBeforeRemoval = $full | ConvertTo-Json -Depth 30 | ConvertFrom-Json -AsHashtable
        Remove-HelmLocalDeployment @{ Docker = 'offline-docker'; Kind = 'offline-kind' } $full $fixture $primary -RemoveFiles | Out-Null
        if ((Test-Path -LiteralPath $allocation) -or (Test-Path -LiteralPath $fixture)) { throw 'Explicit file cleanup retained the exact owned private allocation or run root.' }
        Assert-Fails { $unexpected = Enter-HelmLocalRunLock -Primary $primary -Cluster $cluster; $unexpected.Dispose() } 'Removing the run directory released its active operation lock.'
        [void][IO.Directory]::CreateDirectory($fixture)
        Set-HelmLocalPrivateAcl $fixture
        Write-HelmLocalJson $path $allocatedBeforeRemoval
        Assert-Fails { Initialize-HelmLocalPrivateAllocation -Root $fixture -Manifest $allocatedBeforeRemoval } 'Previously allocated private state was silently regenerated.'
        if (Test-Path -LiteralPath $allocation) { throw 'Missing allocated private state was recreated.' }
    } finally { $cleanupLock.Dispose(); Set-Item Function:Get-HelmLocalNodes $savedNodes }

    Write-Output 'PASS: complete local profile, legacy refusal, credential-free preview, private allocation/PKI retention, missing-key refusal, owned-object recovery, cleanup boundaries and cumulative image receipts.'
} finally {
    # Remove only this exact synthetic fixture, never live deployment resources.
    # If private custody cannot be verified, preserve its D: ownership index too.
    if ($null -ne $allocatedManifestForCleanup -and (Test-Path -LiteralPath $allocatedManifestForCleanup.hosted.privateAllocation.path)) {
        Remove-HelmLocalPrivateAllocation -Root $fixture -Manifest $allocatedManifestForCleanup
    }
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
