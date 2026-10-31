$script:ComposeFailureRoot = $null
function Invoke-ComposeDocker {
    param([string] $Docker, [string[]] $Arguments)
    $output = @(& $Docker @Arguments 2>&1)
    if ($LASTEXITCODE -ne 0) {
        if ($script:ComposeFailureRoot) { Write-ComposeFile (Join-Path $script:ComposeFailureRoot ('docker-failure-' + [Guid]::NewGuid().ToString('N') + '.log')) ($output -join "`n") }
        throw "Docker $($Arguments[0]) failed (exit $LASTEXITCODE). Inspect the private docker-failure log in the deployment directory."
    }
    return ($output -join "`n")
}

function Invoke-Compose {
    param([string] $Docker, [string] $Root, $Manifest, [string[]] $Arguments)
    # Explicit project, files, and selected environment defeat ambient Compose
    # overrides without changing the user's persistent environment or Docker auth.
    $values = @{
        COMPOSE_PROJECT_NAME = $Manifest.project; COMPOSE_PROFILES = $(if ($Manifest.profile -ceq 'hosted-demo') { 'hosted' } else { '' })
        COLOSSUS_DEPLOYMENT_ID = $Manifest.deploymentId; COLOSSUS_SERVER_IMAGE = $Manifest.serverImage
        COLOSSUS_DOCUMENTATION_IMAGE = $Manifest.documentationImage; COLOSSUS_BIND_ADDRESS = $Manifest.bindAddress
        COLOSSUS_BROWSER_PORT = [string]$Manifest.browserPort; COLOSSUS_CONNECTOR_PORT = [string]$Manifest.connectorPort
        COMPOSE_FILE = $null; COMPOSE_ENV_FILES = $null; COMPOSE_IGNORE_ORPHANS = $null
    }
    $saved = @{}
    try {
        foreach ($key in $values.Keys) { $saved[$key] = [Environment]::GetEnvironmentVariable($key, 'Process'); [Environment]::SetEnvironmentVariable($key, $values[$key], 'Process') }
        return Invoke-ComposeDocker $Docker (@('compose','--project-directory',$Root,'--env-file',(Join-Path $Root '.env'),'-f',(Join-Path $Root 'compose.yaml'),'-p',$Manifest.project) + $Arguments)
    } finally {
        foreach ($key in $saved.Keys) {
            if ($null -eq $saved[$key]) { Remove-Item -LiteralPath "Env:$key" -ErrorAction SilentlyContinue }
            else { [Environment]::SetEnvironmentVariable($key, $saved[$key], 'Process') }
        }
    }
}

function Assert-ComposeEngine {
    param([string] $Docker, [string] $Root, $Manifest)
    $info = Invoke-ComposeDocker $Docker @('info','--format','{{json .}}') | ConvertFrom-Json -AsHashtable
    if ($info.OSType -cne 'linux' -or -not $info.ID) { throw 'Use a running Linux Docker engine.' }
    $version = (Invoke-ComposeDocker $Docker @('compose','version','--short')).TrimStart('v')
    if ($version -notmatch '^(\d+)\.(\d+)\.(\d+)' -or [version]$Matches[0] -lt [version]'2.24.0') { throw 'Docker Compose 2.24 or newer is required.' }
    if ($Manifest.engineId -and $Manifest.engineId -cne $info.ID) { throw 'This deployment belongs to a different Docker engine; restore its engine or use explicit backup/restore.' }
    if (-not $Manifest.engineId) { $Manifest.engineId = $info.ID; Write-ComposeJson (Join-Path $Root 'manifest.json') $Manifest }
}

function Get-ComposeResources {
    param([string] $Docker, $Manifest)
    $result = @{ containers = @(); volumes = @(); networks = @() }
    foreach ($kind in @('container','volume','network')) {
        $arguments = switch ($kind) {
            'container' { @('container','ls','--all','--quiet','--filter',"label=com.docker.compose.project=$($Manifest.project)") }
            'volume' { @('volume','ls','--quiet','--filter',"name=^$($Manifest.project)_") }
            'network' { @('network','ls','--quiet','--filter',"name=^$($Manifest.project)_") }
        }
        $names = @((Invoke-ComposeDocker $Docker $arguments) -split '\r?\n' | Where-Object { $_ })
        foreach ($name in $names) {
            $resource = @(Invoke-ComposeDocker $Docker @($kind,'inspect',$name) | ConvertFrom-Json -AsHashtable)[0]
            $labels = if ($kind -ceq 'container') { $resource.Config.Labels } else { $resource.Labels }
            if ($null -eq $labels -or $labels['io.colossus.compose.owner'] -cne $Manifest.deploymentId -or $labels['com.docker.compose.project'] -cne $Manifest.project) { throw 'A Docker resource has a different ownership label; refusing deployment or cleanup.' }
            $key = switch ($kind) { 'container' { 'containers' }; 'volume' { 'volumes' }; 'network' { 'networks' } }
            $result[$key] += $resource
        }
    }
    return $result
}

function Assert-ComposeRetainedVolumes {
    param($Manifest, $Resources)
    if ($Manifest.dataRemoved) { throw 'This deployment was explicitly destroyed; use a fresh name or restore a backup.' }
    if ($Manifest.provisioned) {
        foreach ($name in $Manifest.volumeNames) {
            if (-not @($Resources.volumes | Where-Object Name -CEQ $name).Count) { throw 'A retained data/material volume is missing. Refusing to silently initialize replacement state.' }
        }
    }
}

function Resolve-ComposeImage {
    param([string] $Docker, [string] $Reference, [switch] $LocalImages)
    if ($LocalImages -and $Reference -cmatch '^sha256:[a-f0-9]{64}$') { $repository = '' }
    elseif ($Reference -cnotmatch '^ghcr\.io/obscuritylabs/colossus-(control-plane|documentation)(:v\d+\.\d+\.\d+(-preview\.\d+)?)?(@sha256:[a-f0-9]{64})?$' -or $Reference -notmatch '(:v|@sha256:)') { throw 'Use an exact official release image tag or sha256 digest.' }
    else { $repository = ($Reference -split '[:@]')[0] }
    $inspection = @(& $Docker image inspect $Reference 2>&1)
    if ($LASTEXITCODE -ne 0) {
        if ($LocalImages) { throw 'LocalImages requires both exact selected images to exist in the current engine.' }
        $null = Invoke-ComposeDocker $Docker @('pull',$Reference)
        $inspection = @((Invoke-ComposeDocker $Docker @('image','inspect',$Reference)))
    }
    $image = @(($inspection -join "`n") | ConvertFrom-Json -AsHashtable)[0]
    if ($LocalImages) { return [string]$image.Id }
    $digest = @($image.RepoDigests | Where-Object { $_ -cmatch ('^' + [regex]::Escape($repository) + '@sha256:[a-f0-9]{64}$') }) | Select-Object -First 1
    if (-not $digest) { throw 'The selected release image has no verifiable registry digest.' }
    return [string]$digest
}

function Save-ComposeSelection {
    param([string] $Root, $Manifest)
    $path = Join-Path $Root '.env'
    $temporary = $path + '.' + [Guid]::NewGuid().ToString('N') + '.tmp'
    Write-ComposeFile $temporary (Get-ComposeEnvironmentText $Manifest)
    [IO.File]::Move($temporary, $path, $true)
    $Manifest.files['.env'] = Get-ComposeHash $path
    Write-ComposeJson (Join-Path $Root 'manifest.json') $Manifest
}

function Start-ComposeDeployment {
    param([string] $Docker, [string] $Root, $Manifest, [switch] $DatabaseOnly)
    $resources = Get-ComposeResources $Docker $Manifest
    Assert-ComposeRetainedVolumes $Manifest $resources
    Assert-ComposeCertificates $Root
    Write-Host 'Selecting release images and waiting for deployment readiness...'
    $local = $Manifest.ContainsKey('localImages') -and $Manifest.localImages
    $Manifest.serverImage = Resolve-ComposeImage $Docker $Manifest.serverImage -LocalImages:$local
    $Manifest.documentationImage = Resolve-ComposeImage $Docker $Manifest.documentationImage -LocalImages:$local
    Save-ComposeSelection $Root $Manifest
    $null = Invoke-Compose $Docker $Root $Manifest @('config','--quiet')
    $arguments = @('up','--detach','--wait','--wait-timeout','240')
    if ($DatabaseOnly) { $arguments += 'postgres'; if ($Manifest.profile -ceq 'hosted-demo') { $arguments += 'anchor-postgres' } }
    try { $null = Invoke-Compose $Docker $Root $Manifest $arguments }
    finally {
        # Failed first boots can already contain initialized databases. Retain
        # their ownership too, so a retry cannot silently replace a lost volume.
        $resources = Get-ComposeResources $Docker $Manifest
        if (@($resources.volumes).Count) {
            $Manifest['volumeNames'] = @($resources.volumes | ForEach-Object { $_.Name })
            $Manifest.provisioned = $true
            Write-ComposeJson (Join-Path $Root 'manifest.json') $Manifest
        }
    }
}

function Assert-ComposeUpgrade {
    param([string] $Current, [string] $Selected)
    $pattern = '^v((?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*))(?:-preview\.([1-9][0-9]*))?$'
    if ($Current -cnotmatch $pattern) { throw 'Retained release tag is invalid.' }
    $currentVersion = [version]$Matches[1]; $currentPreview = $Matches[2]
    if ($Selected -cnotmatch $pattern) { throw 'Upgrade requires one exact stable or preview release tag.' }
    $selectedVersion = [version]$Matches[1]; $selectedPreview = $Matches[2]
    $newer = $selectedVersion -gt $currentVersion -or ($selectedVersion -eq $currentVersion -and $currentPreview -and
        (-not $selectedPreview -or [long]$selectedPreview -gt [long]$currentPreview))
    if (-not $newer) { throw 'Upgrade requires a newer release. Restore a backup into a fresh installation for rollback.' }
}

function Stop-ComposeDeployment {
    param([string] $Docker, [string] $Root, $Manifest, [switch] $RemoveData)
    $null = Get-ComposeResources $Docker $Manifest
    $arguments = @('down','--timeout','120')
    if ($RemoveData) { $arguments += '--volumes' }
    $null = Invoke-Compose $Docker $Root $Manifest $arguments
    if ($RemoveData) { $Manifest.dataRemoved = $true; Write-ComposeJson (Join-Path $Root 'manifest.json') $Manifest }
}
