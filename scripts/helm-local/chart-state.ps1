# The chart's retained auth/connector keys are authority, not disposable caches.
function Sync-HelmLocalChartSecrets {
    param($Tools, [string] $Root, [hashtable] $Manifest, [switch] $FirstInstall)
    if (-not $Manifest.hosted.ContainsKey('chartSecrets')) { $Manifest.hosted.chartSecrets = @() }
    $expected = @('colossus-control-plane-state', 'colossus-control-plane-tls')
    foreach ($name in $expected) {
        $record = @($Manifest.hosted.chartSecrets | Where-Object name -CEQ $name)
        if ($record.Count -gt 1) { throw 'Duplicate chart authority receipt.' }
        $raw = (Invoke-HelmLocalKubectl $Tools $Root $Manifest.clusterName @('-n', $Manifest.namespace, 'get', 'secret', $name, '--ignore-not-found', '-o', 'json')).Trim()
        if (-not $raw) {
            if ($record.Count -or -not $FirstInstall) { throw "Retained chart authority $name is missing; restore it instead of generating replacement keys." }
            continue
        }
        $secret = $raw | ConvertFrom-Json -AsHashtable
        if ($secret.metadata.annotations['meta.helm.sh/release-name'] -cne 'colossus-control-plane' -or
            $secret.metadata.annotations['meta.helm.sh/release-namespace'] -cne $Manifest.namespace) { throw 'Chart authority belongs to another release.' }
        $bytes = [Text.Encoding]::UTF8.GetBytes((Get-HelmLocalCanonicalJson $secret.data))
        $hash = Get-HelmLocalSha256 $bytes
        if ($record.Count) {
            if ($record[0].uid -cne $secret.metadata.uid -or $record[0].dataSha256 -cne $hash) { throw "Retained chart authority $name changed; refusing an unreviewed identity rotation." }
            if (-not (Test-Path -LiteralPath $record[0].privateBackup -PathType Leaf) -or (Get-HelmLocalSha256 ([IO.File]::ReadAllBytes($record[0].privateBackup))) -cne $record[0].backupSha256) { throw 'Retained chart authority backup is missing or changed.' }
        } else {
            if (-not $FirstInstall) { throw 'Chart authority was never recorded; retain this deployment for inspection.' }
            $path = Join-Path $Manifest.hosted.privateAllocation.path "chart-secret-$name-$($secret.metadata.uid).json"
            Write-HelmLocalPrivateFile $path $raw
            $Manifest.hosted.chartSecrets += @{ name = $name; uid = $secret.metadata.uid; dataSha256 = $hash; privateBackup = $path; backupSha256 = (Get-HelmLocalSha256 ([IO.File]::ReadAllBytes($path))) }
            Write-HelmLocalJson (Join-Path $Root 'manifest.json') $Manifest
        }
    }
}

function Assert-HelmLocalChartSecretsComplete {
    param([hashtable] $Manifest)
    if (-not $Manifest.hosted.ContainsKey('chartSecrets') -or @($Manifest.hosted.chartSecrets).Count -ne 2) { throw 'Chart authority is incomplete; retain the failed installation for inspection.' }
}

function Reset-HelmLocalRemovedGeneration {
    param([string] $Root, [hashtable] $Manifest)
    if (-not $Manifest.ContainsKey('clusterRemoved') -or -not $Manifest.clusterRemoved) { return }
    if (-not $Manifest.hosted.ContainsKey('retiredGenerations')) { $Manifest.hosted.retiredGenerations = @() }
    $retired = @{ resources = @($Manifest.hosted.resources); retiredAtUtc = [DateTime]::UtcNow.ToString('o') }
    foreach ($key in @('volumes', 'chartSecrets', 'project')) { if ($Manifest.hosted.ContainsKey($key)) { $retired[$key] = $Manifest.hosted[$key] } }
    $Manifest.hosted.retiredGenerations += $retired
    $Manifest.hosted.resources = @(); $Manifest.hosted.volumes = @{}; $Manifest.hosted.chartSecrets = @()
    $Manifest.hosted.project = @{ stage = 'planned'; id = 'local' }; $Manifest.hosted.stage = 'planned'
    $Manifest.hosted.Remove('chartInstallIntent')
    $Manifest.clusterRemoved = $false
    Write-HelmLocalJson (Join-Path $Root 'manifest.json') $Manifest
}
