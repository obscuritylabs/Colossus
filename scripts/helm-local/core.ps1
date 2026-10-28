Set-StrictMode -Version Latest
$script:HelmLocalHelperRoot = $PSScriptRoot
. (Join-Path $PSScriptRoot 'private.ps1')
. (Join-Path $PSScriptRoot 'hosted.ps1')
. (Join-Path $PSScriptRoot 'chart-state.ps1')

function Resolve-HelmLocalTool {
    param([string] $Name, [string] $Explicit, [switch] $Optional)
    $candidate = if ($Explicit) { $Explicit } else { $Name }
    $command = Get-Command $candidate -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($null -eq $command) {
        if ($Optional -and -not $Explicit) { return $null }
        throw "Install $Name on PATH or pass its executable path."
    }
    return $command.Source
}

function Invoke-HelmLocalNative {
    param([string] $Executable, [string[]] $Arguments)
    $output = @(& $Executable @Arguments 2>&1)
    if ($LASTEXITCODE -ne 0) {
        # Do not interpolate native output: callers may be handling Secrets.
        throw "$([IO.Path]::GetFileName($Executable)) $($Arguments[0]) failed (exit $LASTEXITCODE). Deployment files are retained."
    }
    return ($output -join [Environment]::NewLine)
}

function Assert-HelmLocalBoundary {
    param([string] $Root, [string] $Primary)
    $parent = [IO.Path]::GetFullPath((Join-Path $Primary '.local/tests')).TrimEnd('\', '/')
    $actual = [IO.Path]::GetFullPath($Root).TrimEnd('\', '/')
    if (-not [string]::Equals([IO.Path]::GetDirectoryName($actual), $parent, [StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetFileName($actual) -cnotmatch '^helm-[a-z][a-z0-9]*(?:-[a-z0-9]+)*$') {
        throw 'The run directory is outside the primary checkout test boundary.'
    }
    $cursor = $actual
    while ($cursor) {
        if (Test-Path -LiteralPath $cursor) {
            $item = Get-Item -LiteralPath $cursor -Force
            if (-not $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
                throw 'A run directory or ancestor is a file/reparse point; refusing access.'
            }
        }
        $cursor = [IO.Path]::GetDirectoryName($cursor)
    }
}

function Set-HelmLocalPrivateAcl {
    param([string] $Root)
    $acl = [Security.AccessControl.DirectorySecurity]::new()
    $acl.SetAccessRuleProtection($true, $false)
    $user = [Security.Principal.WindowsIdentity]::GetCurrent().User
    $acl.SetOwner($user)
    foreach ($sid in @($user, [Security.Principal.SecurityIdentifier]::new('S-1-5-18'))) {
        $rule = [Security.AccessControl.FileSystemAccessRule]::new($sid, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow')
        [void]$acl.AddAccessRule($rule)
    }
    Set-Acl -LiteralPath $Root -AclObject $acl
    Assert-HelmLocalPrivateAcl $Root
}

function Assert-HelmLocalPrivateAcl {
    param([string] $Root)
    $acl = Get-Acl -LiteralPath $Root
    $allowed = @([Security.Principal.WindowsIdentity]::GetCurrent().User.Value, 'S-1-5-18')
    $owner = ([Security.Principal.NTAccount]$acl.Owner).Translate([Security.Principal.SecurityIdentifier]).Value
    if ($owner -cne $allowed[0]) { throw 'The private directory is not owned by the current user.' }
    if (-not $acl.AreAccessRulesProtected) { throw 'The run directory must have its own private ACL.' }
    foreach ($rule in $acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])) {
        if ($rule.AccessControlType -eq 'Allow' -and $rule.IdentityReference.Value -notin $allowed) {
            throw 'The run directory grants access beyond the current user and SYSTEM.'
        }
    }
}

function Read-HelmLocalManifest {
    param([string] $Path, [string] $Root, [string] $Cluster, [string] $Primary)
    if (-not (Test-Path -LiteralPath $Root)) { return $null }
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw 'Existing run directory has no ownership manifest; use another -Name.' }
    if ((Get-Item -LiteralPath $Path -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Ownership manifest is a reparse point.' }
    $manifest = Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json -AsHashtable
    if ($manifest.schemaVersion -notin @(2, 3) -or $manifest.purpose -cne 'ColossusLocalHelmHarness' -or
        -not [string]::Equals([string]$manifest.rootPath, $Root, [StringComparison]::OrdinalIgnoreCase) -or
        -not [string]::Equals([string]$manifest.primaryRoot, $Primary, [StringComparison]::OrdinalIgnoreCase) -or
        $manifest.clusterName -cne $Cluster -or $manifest.namespace -cne $Cluster -or
        [int]$manifest.port -lt 1 -or [int]$manifest.port -gt 65535 -or
        -not $manifest.sourcePath -or -not $manifest.sourceCommit -or
        -not $manifest.ContainsKey('nodes') -or -not $manifest.ContainsKey('images')) {
        throw 'Ownership manifest does not describe this exact harness deployment; refusing reuse.'
    }
    if ($manifest.schemaVersion -eq 3) { Assert-HelmLocalFullProfile $manifest }
    return $manifest
}

function Write-HelmLocalJson {
    param([string] $Path, $Value)
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($Value | ConvertTo-Json -Depth 40) + "`n")
    $temporary = $Path + '.' + [Guid]::NewGuid().ToString('N') + '.tmp'
    $stream = [IO.FileStream]::new($temporary, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $stream.Write($bytes); $stream.Flush($true) } finally { $stream.Dispose() }
    # Same-directory replacement keeps the previous complete receipt on failure.
    [IO.File]::Move($temporary, $Path, $true)
}

function Enter-HelmLocalRunLock {
    param([string] $Primary, [string] $Cluster)
    $directory = [IO.Path]::GetFullPath((Join-Path $Primary '.local/tests/helm-locks'))
    Assert-HelmLocalPrivatePath $directory
    if (-not (Test-Path -LiteralPath $directory)) {
        [void][IO.Directory]::CreateDirectory($directory)
        Set-HelmLocalPrivateAcl $directory
    }
    Assert-HelmLocalPrivateAcl $directory
    $identity = [Text.Encoding]::UTF8.GetBytes([IO.Path]::GetFullPath($Primary).ToLowerInvariant() + [char]0 + $Cluster)
    $name = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($identity)).ToLowerInvariant() + '.lock'
    $path = Join-Path $directory $name
    if ((Test-Path -LiteralPath $path) -and ((Get-Item -LiteralPath $path -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Operation lock is a reparse point.' }
    try {
        # Outside the owned run directory, so down can remove that directory while
        # this handle excludes every competing up/test/down. The OS removes it on close.
        return [IO.FileStream]::new($path, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None, 4096, [IO.FileOptions]::DeleteOnClose)
    } catch [IO.IOException] { throw 'This local deployment is busy. Wait for its current operation to finish.' }
}

function Save-HelmLocalEnvironment {
    $saved = @{}
    foreach ($key in @('DOCKER_HOST', 'DOCKER_CONTEXT', 'DOCKER_CONFIG', 'KUBECONFIG', 'HELM_CACHE_HOME', 'HELM_CONFIG_HOME', 'HELM_DATA_HOME', 'TEMP', 'TMP', 'KIND_EXPERIMENTAL_PROVIDER')) {
        $saved[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
    }
    return $saved
}

function Restore-HelmLocalEnvironment {
    param([hashtable] $Saved)
    foreach ($key in $Saved.Keys) { [Environment]::SetEnvironmentVariable($key, $Saved[$key], 'Process') }
}

function Set-HelmLocalEnvironment {
    param([string] $Root)
    foreach ($directory in @('docker', 'helm/cache', 'helm/config', 'helm/data', 'tmp', 'logs', 'packages', 'kube-cache')) {
        [void][IO.Directory]::CreateDirectory((Join-Path $Root $directory))
    }
    $env:DOCKER_HOST = 'npipe:////./pipe/dockerDesktopLinuxEngine'
    [Environment]::SetEnvironmentVariable('DOCKER_CONTEXT', $null, 'Process')
    $env:DOCKER_CONFIG = Join-Path $Root 'docker'
    $env:KUBECONFIG = Join-Path (Get-HelmLocalPrivateRoot $Root) 'kubeconfig'
    $env:HELM_CACHE_HOME = Join-Path $Root 'helm/cache'
    $env:HELM_CONFIG_HOME = Join-Path $Root 'helm/config'
    $env:HELM_DATA_HOME = Join-Path $Root 'helm/data'
    $env:TEMP = Join-Path $Root 'tmp'
    $env:TMP = $env:TEMP
    $env:KIND_EXPERIMENTAL_PROVIDER = 'docker'
}

function Get-HelmLocalNodes {
    param($Tools, [string] $Cluster)
    $ids = (Invoke-HelmLocalNative $Tools.Docker @('ps', '-aq', '--no-trunc', '--filter', "label=io.x-k8s.kind.cluster=$Cluster")).Trim()
    if (-not $ids) { return @() }
    $nodes = @()
    foreach ($id in ($ids -split '\r?\n')) {
        $node = @(Invoke-HelmLocalNative $Tools.Docker @('inspect', $id) | ConvertFrom-Json)[0]
        if ($node.Config.Labels.'io.x-k8s.kind.cluster' -cne $Cluster) { throw 'Cluster container label changed.' }
        $nodes += @{ id = [string]$node.Id; name = ([string]$node.Name).TrimStart('/') }
    }
    return $nodes
}

function Assert-HelmLocalNodes {
    param($Actual, $Expected)
    $actualIds = @($Actual | ForEach-Object { $_.id } | Sort-Object)
    $expectedIds = @($Expected | ForEach-Object { $_.id } | Sort-Object)
    if ($actualIds.Count -ne $expectedIds.Count -or ($actualIds -join ',') -cne ($expectedIds -join ',')) {
        throw 'Cluster container IDs do not match the ownership receipt; refusing access.'
    }
}

function Convert-HelmLocalImage {
    param([string] $Reference, [string] $PullPolicy)
    if ($Reference -cnotmatch '^[a-zA-Z0-9][a-zA-Z0-9._/+:@-]*$') { throw 'Invalid image reference.' }
    $parts = $Reference.Split('@')
    if ($parts.Count -gt 2) { throw 'Invalid image digest.' }
    $name = $parts[0]
    $digest = if ($parts.Count -eq 2) { $parts[1] } else { '' }
    if ($digest -and $digest -cnotmatch '^sha256:[a-f0-9]{64}$') { throw 'Only exact sha256 image digests are supported.' }
    $tag = 'latest'
    if ($name.LastIndexOf(':') -gt $name.LastIndexOf('/')) {
        $tag = $name.Substring($name.LastIndexOf(':') + 1)
        $name = $name.Substring(0, $name.LastIndexOf(':'))
    }
    $components = $name.Split('/')
    if ($components.Count -gt 1 -and ($components[0].Contains('.') -or $components[0].Contains(':') -or $components[0] -eq 'localhost')) {
        $registry = $components[0]; $repository = $components[1..($components.Count - 1)] -join '/'
    } else {
        $registry = 'docker.io'; $repository = if ($components.Count -eq 1) { "library/$name" } else { $name }
    }
    return @{ registry = $registry; repository = $repository; tag = $tag; digest = $digest; pullPolicy = $PullPolicy }
}

function Get-HelmLocalImageReceipt {
    param($Tools, [string] $Reference, [string] $Root)
    $present = (Invoke-HelmLocalNative $Tools.Docker @('image', 'ls', '--quiet', '--no-trunc', '--filter', "reference=$Reference")).Trim()
    if (-not $present) { return @{ reference = $Reference; id = ''; owned = $false } }
    $image = @(Invoke-HelmLocalNative $Tools.Docker @('image', 'inspect', $Reference) | ConvertFrom-Json -AsHashtable)[0]
    $labels = $image.Config.Labels
    $owned = $null -ne $labels -and $labels['com.colossus.test.purpose'] -ceq 'ColossusLocalHelm' -and
        [string]::Equals([string]$labels['com.colossus.test.root'], $Root, [StringComparison]::OrdinalIgnoreCase)
    return @{ reference = $Reference; id = [string]$image.Id; owned = [bool]$owned }
}

function Merge-HelmLocalImageReceipts {
    param($Previous, $Selected)
    # Keep past owned artifacts even after another tag is selected for deployment.
    # Exact reference+ID pairs are distinct: moving a tag must retain the earlier
    # receipt so cleanup refuses to discard evidence of the previous image.
    $receipts = [Collections.Generic.List[object]]::new()
    $indices = [Collections.Generic.Dictionary[string, int]]::new([StringComparer]::Ordinal)
    foreach ($image in (@($Previous | Where-Object { $null -ne $_ -and $_.owned }) + @($Selected | Where-Object { $null -ne $_ }))) {
        $key = [string]$image.reference + [char]0 + [string]$image.id
        if ($indices.ContainsKey($key)) { continue }
        $indices.Add($key, $receipts.Count)
        $receipts.Add(@{ reference = [string]$image.reference; id = [string]$image.id; owned = [bool]$image.owned })
    }
    return $receipts.ToArray()
}

function Invoke-HelmLocalKubectl {
    param($Tools, [string] $Root, [string] $Cluster, [string[]] $Arguments)
    return Invoke-HelmLocalNative $Tools.Kubectl (@('--kubeconfig', (Join-Path (Get-HelmLocalPrivateRoot $Root) 'kubeconfig'), '--context', "kind-$Cluster", '--cache-dir', (Join-Path $Root 'kube-cache')) + $Arguments)
}

function New-HelmLocalBrowserCertificate {
    param([string] $Root, $Manifest = $null)
    if ($null -eq $Manifest) { $Manifest = Get-Content -LiteralPath (Join-Path $Root 'manifest.json') -Raw | ConvertFrom-Json -AsHashtable }
    $private = Get-HelmLocalPrivateRoot $Root
    $certPath = Join-Path $private 'browser-cert.pem'
    $keyPath = Join-Path $private 'browser-key.pem'
    if ($Manifest.schemaVersion -eq 3 -and $Manifest.ContainsKey('browserTls')) {
        foreach ($pair in @(@($certPath, 'certificateSha256'), @($keyPath, 'keySha256'))) {
            if (-not (Test-Path -LiteralPath $pair[0] -PathType Leaf) -or (Get-FileHash -LiteralPath $pair[0] -Algorithm SHA256).Hash.ToLowerInvariant() -cne $Manifest.browserTls[$pair[1]]) { throw 'Retained browser TLS material is missing or changed; refusing replacement.' }
        }
        return
    }
    if ((Test-Path -LiteralPath $certPath) -or (Test-Path -LiteralPath $keyPath)) {
        if (-not ((Test-Path -LiteralPath $certPath) -and (Test-Path -LiteralPath $keyPath))) { throw 'Browser TLS material is incomplete; retain and inspect the deployment.' }
        if ($Manifest.schemaVersion -eq 3) { throw 'Unrecorded browser TLS material requires inspection; refusing adoption.' }
        return
    }
    $rsa = [Security.Cryptography.RSA]::Create(2048)
    try {
        $request = [Security.Cryptography.X509Certificates.CertificateRequest]::new('CN=colossus.localhost', $rsa, [Security.Cryptography.HashAlgorithmName]::SHA256, [Security.Cryptography.RSASignaturePadding]::Pkcs1)
        $san = [Security.Cryptography.X509Certificates.SubjectAlternativeNameBuilder]::new()
        $san.AddDnsName('colossus.localhost')
        $request.CertificateExtensions.Add($san.Build())
        $request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509BasicConstraintsExtension]::new($false, $false, 0, $true))
        $request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509KeyUsageExtension]::new('DigitalSignature,KeyEncipherment', $true))
        $eku = [Security.Cryptography.OidCollection]::new()
        [void]$eku.Add([Security.Cryptography.Oid]::new('1.3.6.1.5.5.7.3.1'))
        $request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension]::new($eku, $false))
        $cert = $request.CreateSelfSigned([DateTimeOffset]::UtcNow.AddMinutes(-5), [DateTimeOffset]::UtcNow.AddDays(30))
        try {
            [IO.File]::WriteAllText($certPath, $cert.ExportCertificatePem())
            [IO.File]::WriteAllText($keyPath, $rsa.ExportPkcs8PrivateKeyPem())
        } finally { $cert.Dispose() }
    } finally { $rsa.Dispose() }
    if ($Manifest.schemaVersion -eq 3) {
        $Manifest.browserTls = @{
            certificateSha256 = (Get-FileHash -LiteralPath $certPath -Algorithm SHA256).Hash.ToLowerInvariant()
            keySha256 = (Get-FileHash -LiteralPath $keyPath -Algorithm SHA256).Hash.ToLowerInvariant()
        }
        Write-HelmLocalJson (Join-Path $Root 'manifest.json') $Manifest
    }
}

function Start-HelmLocalDeployment {
    param($Tools, [string] $Source, [string] $Primary, [string] $Root, [string] $Cluster, [string] $Name, [int] $Port, $Manifest, [string] $ServerImage, [string] $DocsImage, [switch] $LoadImages)
    Assert-HelmLocalFullProfile $Manifest
    $private = Assert-HelmLocalPrivateAllocation $Root $Manifest
    $chart = Join-Path $Source 'deploy/control-plane/chart'
    $chartText = Get-Content -LiteralPath (Join-Path $chart 'Chart.yaml') -Raw
    $version = [regex]::Match($chartText, '(?m)^appVersion:\s*"?([^"\s]+)"?\s*$').Groups[1].Value
    if (-not $version) { throw 'Chart appVersion is missing.' }
    if (-not $ServerImage) { $ServerImage = "ghcr.io/obscuritylabs/colossus-control-plane:v$version" }
    if (-not $DocsImage) { $DocsImage = "ghcr.io/obscuritylabs/colossus-documentation:v$version" }
    $policy = if ($LoadImages) { 'Never' } else { 'IfNotPresent' }
    $serverValues = Convert-HelmLocalImage $ServerImage $policy
    $docsValues = Convert-HelmLocalImage $DocsImage $policy
    $nodes = @(Get-HelmLocalNodes $Tools $Cluster)
    if ($null -eq $Manifest -and $nodes.Count) { throw 'A cluster already uses this name without an ownership receipt.' }
    if ($null -ne $Manifest -and $nodes.Count) { Assert-HelmLocalNodes $nodes $Manifest.nodes }
    if ($nodes.Count -and $Manifest.hosted.ContainsKey('chartInstallIntent')) {
        Sync-HelmLocalChartSecrets $Tools $Root $Manifest
        Assert-HelmLocalChartSecretsComplete $Manifest
    }
    if (-not $nodes.Count) { Reset-HelmLocalRemovedGeneration $Root $Manifest }
    $images = @((Get-HelmLocalImageReceipt $Tools $ServerImage $Root), (Get-HelmLocalImageReceipt $Tools $DocsImage $Root))
    if ($LoadImages -and @($images | Where-Object { -not $_.id }).Count) { throw '-LoadImages requires both exact image references to exist in the local Linux Docker engine.' }
    $previousImages = if ($null -ne $Manifest) { @($Manifest.images) } else { @() }
    $imageReceipts = @(Merge-HelmLocalImageReceipts $previousImages $images)
    $git = Resolve-HelmLocalTool 'git' ''
    # Retain profile, material, resource identities and previous image receipts.
    $Manifest.sourcePath = $Source
    $Manifest.sourceCommit = (Invoke-HelmLocalNative $git @('-C', $Source, 'rev-parse', 'HEAD')).Trim()
    $Manifest.sourceDirty = [bool](Invoke-HelmLocalNative $git @('-C', $Source, 'status', '--porcelain', '--untracked-files=normal')).Trim()
    $Manifest.chartAppVersion = $version
    $Manifest.browserHost = 'colossus.localhost'; $Manifest.connectorHost = 'runtimes.colossus.localhost'
    $Manifest.serverImage = $ServerImage; $Manifest.documentationImage = $DocsImage
    $Manifest.images = $imageReceipts; $Manifest.selectedImages = $images; $Manifest.nodes = $nodes
    Write-HelmLocalJson (Join-Path $Root 'manifest.json') $Manifest
    $kindConfig = @"
kind: Cluster
apiVersion: kind.x-k8s.io/v1alpha4
name: $Cluster
nodes:
  - role: control-plane
    image: kindest/node:v1.34.11@sha256:44e222ee2132dab25ff87301682f89eb82c7880ea3a1bf543bfe9708fd08d67d
    extraPortMappings:
      - containerPort: 30443
        hostPort: $Port
        listenAddress: 127.0.0.1
        protocol: TCP
"@
    [IO.File]::WriteAllText((Join-Path $Root 'kind.yaml'), $kindConfig)
    if (-not $nodes.Count) {
        try {
            $log = Invoke-HelmLocalNative $Tools.Kind @('create', 'cluster', '--name', $Cluster, '--config', (Join-Path $Root 'kind.yaml'), '--kubeconfig', (Join-Path $private 'kubeconfig'), '--wait', '180s')
            [IO.File]::WriteAllText((Join-Path $Root 'logs/kind-up.log'), $log)
        } finally {
            # There were no cluster containers before this exact create attempt.
            $Manifest.nodes = @(Get-HelmLocalNodes $Tools $Cluster)
            Write-HelmLocalJson (Join-Path $Root 'manifest.json') $Manifest
        }
    }
    if ($LoadImages) {
        $null = Invoke-HelmLocalNative $Tools.Kind @('load', 'docker-image', '--name', $Cluster, $ServerImage, $DocsImage)
    }
    $package = Join-Path $Root 'packages/gateway-helm-v1.9.3.tgz'
    $cache = Join-Path $Primary '.local/tests/helm-local/packages/gateway-helm-v1.9.3.tgz'
    if (-not (Test-Path -LiteralPath $package)) {
        if (Test-Path -LiteralPath $cache) { Copy-Item -LiteralPath $cache -Destination $package }
        else { $null = Invoke-HelmLocalNative $Tools.Helm @('pull', 'oci://docker.io/envoyproxy/gateway-helm', '--version', 'v1.9.3', '--destination', (Join-Path $Root 'packages')) }
    }
    if ((Get-FileHash -LiteralPath $package -Algorithm SHA256).Hash -cne '1A152513ADBCDB0527D7BBE3EA9B090ABEB87E4EC9F34B2C18AC8C37F3177541') { throw 'Envoy Gateway package checksum mismatch.' }
    $log = Invoke-HelmLocalNative $Tools.Helm @('upgrade', '--install', 'envoy-gateway', $package, '--kubeconfig', (Join-Path $private 'kubeconfig'), '--namespace', 'envoy-gateway-system', '--create-namespace', '--wait', '--timeout', '5m')
    [IO.File]::WriteAllText((Join-Path $Root 'logs/gateway-install.log'), $log)
    $null = Invoke-HelmLocalKubectl $Tools $Root $Cluster @('wait', '--for=condition=Established', '--timeout=120s', 'crd/gateways.gateway.networking.k8s.io', 'crd/tlsroutes.gateway.networking.k8s.io', 'crd/envoyproxies.gateway.envoyproxy.io')
    $ns = @{ apiVersion = 'v1'; kind = 'Namespace'; metadata = @{ name = $Cluster; labels = @{ 'colossus.ai/local-deployment' = $Manifest.deploymentId } } }
    Write-HelmLocalJson (Join-Path $Root 'namespace.json') $ns
    Ensure-HelmLocalHostedObject $Tools $Root $Manifest $ns
    New-HelmLocalBrowserCertificate $Root $Manifest
    $tls = @{
        apiVersion = 'v1'; kind = 'Secret'; type = 'kubernetes.io/tls'; metadata = @{ name = 'colossus-browser-tls'; namespace = $Cluster; labels = @{ 'colossus.ai/local-deployment' = $Manifest.deploymentId } }
        data = @{ 'tls.crt' = [Convert]::ToBase64String([IO.File]::ReadAllBytes((Join-Path $private 'browser-cert.pem'))); 'tls.key' = [Convert]::ToBase64String([IO.File]::ReadAllBytes((Join-Path $private 'browser-key.pem'))) }
    }
    Ensure-HelmLocalHostedObject $Tools $Root $Manifest $tls
    $gateway = @"
apiVersion: gateway.networking.k8s.io/v1
kind: GatewayClass
metadata:
  name: $Cluster
spec:
  controllerName: gateway.envoyproxy.io/gatewayclass-controller
---
apiVersion: gateway.envoyproxy.io/v1alpha1
kind: EnvoyProxy
metadata:
  name: $Cluster-proxy
  namespace: $Cluster
spec:
  concurrency: 2
  provider:
    type: Kubernetes
    kubernetes:
      envoyDeployment:
        replicas: 1
      envoyService:
        type: NodePort
        externalTrafficPolicy: Cluster
        patch:
          type: StrategicMerge
          value:
            spec:
              ports:
                - port: 443
                  nodePort: 30443
---
apiVersion: gateway.networking.k8s.io/v1
kind: Gateway
metadata:
  name: colossus
  namespace: $Cluster
spec:
  gatewayClassName: $Cluster
  infrastructure:
    parametersRef:
      group: gateway.envoyproxy.io
      kind: EnvoyProxy
      name: $Cluster-proxy
  listeners:
    - name: browser
      hostname: colossus.localhost
      protocol: HTTPS
      port: 443
      tls:
        mode: Terminate
        certificateRefs:
          - name: colossus-browser-tls
    - name: connector
      hostname: runtimes.colossus.localhost
      protocol: TLS
      port: 443
      tls:
        mode: Passthrough
"@
    [IO.File]::WriteAllText((Join-Path $Root 'gateway.yaml'), $gateway)
    $null = Invoke-HelmLocalKubectl $Tools $Root $Cluster @('apply', '-f', (Join-Path $Root 'gateway.yaml'))
    $values = @{
        browserHost = 'colossus.localhost'; connectorHost = 'runtimes.colossus.localhost'; replicaCount = 3
        image = $serverValues; documentation = @{ replicaCount = 1; image = $docsValues }
        autoscaling = @{ enabled = $false }; topologySpread = @{ enabled = $false }; podDisruptionBudget = @{ enabled = $false }
        routing = @{ mode = 'gateway-api'; publicPort = $Port; gatewayAPI = @{ name = 'colossus' } }
    }
    $hostedValues = Initialize-HelmLocalHosted -Tools $Tools -Source $Source -Primary $Primary -Root $Root -Cluster $Cluster -Manifest $Manifest
    foreach ($key in $hostedValues.Keys) { $values[$key] = $hostedValues[$key] }
    Write-HelmLocalJson (Join-Path $Root 'values.json') $values
    $firstInstall = -not $Manifest.hosted.ContainsKey('chartInstallIntent')
    if ($firstInstall) {
        $Manifest.hosted.chartInstallIntent = @{ recordedAtUtc = [DateTime]::UtcNow.ToString('o') }
        Write-HelmLocalJson (Join-Path $Root 'manifest.json') $Manifest
    }
    try {
        $log = Invoke-HelmLocalNative $Tools.Helm @('upgrade', '--install', 'colossus-control-plane', $chart, '--kubeconfig', (Join-Path $private 'kubeconfig'), '--namespace', $Cluster, '-f', (Join-Path $Root 'values.json'), '--wait', '--timeout', '10m')
    } finally { Sync-HelmLocalChartSecrets $Tools $Root $Manifest -FirstInstall:$firstInstall }
    Assert-HelmLocalChartSecretsComplete $Manifest
    [IO.File]::WriteAllText((Join-Path $Root 'logs/helm-install.log'), $log)
    $null = Invoke-HelmLocalKubectl $Tools $Root $Cluster @('-n', $Cluster, 'wait', '--for=condition=Programmed', 'gateway/colossus', '--timeout=180s')
    return $Manifest
}

function Test-HelmLocalDeployment {
    param($Tools, $Manifest, [string] $Root, [switch] $InitializeHosted)
    Assert-HelmLocalFullProfile $Manifest
    $private = Assert-HelmLocalPrivateAllocation $Root $Manifest
    Sync-HelmLocalChartSecrets $Tools $Root $Manifest
    Assert-HelmLocalChartSecretsComplete $Manifest
    Assert-HelmLocalNodes @(Get-HelmLocalNodes $Tools $Manifest.clusterName) $Manifest.nodes
    $port = [int]$Manifest.port
    $base = if ($port -eq 443) { 'https://colossus.localhost' } else { "https://colossus.localhost:$port" }
    $cookie = Join-Path $private 'smoke-cookie.txt'
    $request = {
        param([string] $Path, [string] $Output, [string[]] $Extra = @())
        $requestArgs = @('--silent', '--show-error', '--max-time', '30', '--resolve', "colossus.localhost:${port}:127.0.0.1", '--cacert', (Join-Path $private 'browser-cert.pem'), '--output', $Output, '--write-out', '%{http_code}') + $Extra + @($base + $Path)
        return [int](Invoke-HelmLocalNative $Tools.Curl $requestArgs)
    }
    $results = [ordered]@{ url = $base; sourceCommit = $Manifest.sourceCommit }
    foreach ($health in @('live', 'ready')) {
        $output = Join-Path $Root "logs/health-$health.txt"
        $status = & $request "/health/$health" $output
        if ($status -ne 200 -or (Get-Content -LiteralPath $output -Raw).Trim() -cne 'ok') { throw "Health check $health failed." }
        $results["health_$health"] = $status
    }
    $configPath = Join-Path $Root 'logs/auth-config.json'
    if ((& $request '/api/auth/config' $configPath) -ne 200 -or -not (Get-Content -LiteralPath $configPath -Raw | ConvertFrom-Json).local_enabled) { throw 'Local authentication is unavailable.' }
    if ((& $request '/api/me' (Join-Path $Root 'logs/anonymous.json')) -ne 403) { throw 'Anonymous identity request was not denied.' }
    $secret = Invoke-HelmLocalKubectl $Tools $Root $Manifest.clusterName @('-n', $Manifest.namespace, 'get', 'secret', 'colossus-control-plane-state', '-o', 'json') | ConvertFrom-Json
    $password = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($secret.data.'bootstrap-password'))
    [IO.File]::WriteAllText((Join-Path $private 'admin-login.txt'), "URL: $base`r`nUsername: admin`r`nPassword: $password`r`n")
    $loginPath = Join-Path $private 'login-request.json'
    Write-HelmLocalJson $loginPath @{ username = 'admin'; password = $password }
    $status = & $request '/auth/local' (Join-Path $private 'login-response.txt') @('--cookie-jar', $cookie, '--header', 'Content-Type: application/json', '--header', "Origin: $base", '--header', 'X-Colossus-CSRF: 1', '--data-binary', "@$loginPath")
    if ($status -ne 204) { throw "Bootstrap sign-in failed (HTTP $status). If its password was changed, use the retained account credentials for manual testing." }
    $identity = Join-Path $private 'identity.json'
    if ((& $request '/api/me' $identity @('--cookie', $cookie)) -ne 200 -or -not (Get-Content -LiteralPath $identity -Raw | ConvertFrom-Json).user.is_admin) { throw 'Authenticated administrator check failed.' }
    $results.local_login = $status
    foreach ($path in @('/', '/docs/')) {
        $output = Join-Path $Root $(if ($path -eq '/') { 'logs/index.html' } else { 'logs/docs.html' })
        if ((& $request $path $output) -ne 200 -or (Get-Content -LiteralPath $output -Raw) -notmatch '<html') { throw "Page check failed: $path" }
    }
    $searchPath = Join-Path $Root 'logs/search.json'
    if ((& $request '/docs/search.json' $searchPath) -ne 200) { throw 'Documentation search request failed.' }
    $search = Get-Content -LiteralPath $searchPath -Raw | ConvertFrom-Json
    if (@($search.items).Count -le 100) { throw 'Documentation search index is incomplete.' }
    foreach ($item in $search.items) {
        $location = [Uri]::new([Uri]($base + '/docs/'), [string]$item.location)
        if ($location.Host -cne 'colossus.localhost' -or $location.Port -ne $port -or -not $location.AbsolutePath.StartsWith('/docs/')) { throw 'Documentation search leaves the deployment origin.' }
    }
    $results.documentation_search_entries = @($search.items).Count
    if ($Tools.Node) {
        $tls = Invoke-HelmLocalKubectl $Tools $Root $Manifest.clusterName @('-n', $Manifest.namespace, 'get', 'secret', 'colossus-control-plane-tls', '-o', 'json') | ConvertFrom-Json
        foreach ($entry in @(@('ca.pem', 'connector-ca.pem'), @('server.pem', 'connector-server.pem'))) {
            [IO.File]::WriteAllBytes((Join-Path $private $entry[1]), [Convert]::FromBase64String($tls.data.($entry[0])))
        }
        $null = Invoke-HelmLocalNative $Tools.Node @((Join-Path $script:HelmLocalHelperRoot 'connector-probe.cjs'), $Root, [string]$port)
        $results.connector_tls = 'verified'
    } else { throw 'Node.js is required for complete local hosting verification.' }
    if ($InitializeHosted) { $null = Invoke-HelmLocalNative $Tools.Node @((Join-Path $script:HelmLocalHelperRoot 'hosted-probe.cjs'), $Root, [string]$port, 'initialize') }
    $null = Invoke-HelmLocalNative $Tools.Node @((Join-Path $script:HelmLocalHelperRoot 'hosted-probe.cjs'), $Root, [string]$port, 'verify')
    $results.hosted = 'verified'; $results.profile = 'full-hosted-v1'
    $Manifest = Get-Content -LiteralPath (Join-Path $Root 'manifest.json') -Raw | ConvertFrom-Json -AsHashtable
    $Manifest.hosted.stage = 'ready'
    Write-HelmLocalJson (Join-Path $Root 'manifest.json') $Manifest
    $results.verifiedAtUtc = [DateTime]::UtcNow.ToString('o')
    Write-HelmLocalJson (Join-Path $Root 'verification.json') $results
    Write-Output "Local deployment verified: $base"
    Write-Output "Credentials: $(Join-Path $private 'admin-login.txt') (owner-private; password not printed)."
    Write-Output "Receipt: $(Join-Path $Root 'verification.json')"
}

function Assert-HelmLocalNoChildReparse {
    param([string] $Root)
    $pending = [Collections.Generic.Stack[string]]::new()
    $pending.Push($Root)
    while ($pending.Count) {
        foreach ($entry in Get-ChildItem -LiteralPath $pending.Pop() -Force) {
            if ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Run directory contains a reparse point; files are retained.' }
            if ($entry.PSIsContainer) { $pending.Push($entry.FullName) }
        }
    }
}

function Remove-HelmLocalDeployment {
    param($Tools, $Manifest, [string] $Root, [string] $Primary, [switch] $RemoveImages, [switch] $RemoveFiles)
    $owned = @($Manifest.images | Where-Object owned)
    if ($RemoveFiles -and $Manifest.schemaVersion -eq 3 -and $Manifest.hosted.privateAllocation.allocated -and -not $Manifest.hosted.privateAllocation.removed) {
        $null = Assert-HelmLocalPrivateAllocation $Root $Manifest
        Assert-HelmLocalNoChildReparse ([string]$Manifest.hosted.privateAllocation.path)
    }
    if ($RemoveFiles -and $owned.Count -and -not $RemoveImages) { throw 'Use -RemoveImages with -RemoveFiles to retain image ownership receipts until cleanup succeeds.' }
    $actual = @(Get-HelmLocalNodes $Tools $Manifest.clusterName)
    if ($actual.Count) { Assert-HelmLocalNodes $actual $Manifest.nodes }
    $presentImages = @()
    if ($RemoveImages) {
        foreach ($image in $owned) {
            $current = Get-HelmLocalImageReceipt $Tools $image.reference $Root
            if (-not $current.id) {
                $remainingIds = (Invoke-HelmLocalNative $Tools.Docker @('image', 'ls', '--quiet', '--no-trunc')) -split '\r?\n'
                if ([string]$image.id -cin $remainingIds) {
                    throw "Recorded image $($image.id) still exists without its original reference $($image.reference). No resources were removed. The ownership receipt and files are retained; review that image before retrying cleanup."
                }
                continue
            }
            if (-not $current.owned -or $current.id -cne $image.id) {
                throw "Image $($image.reference) no longer has its recorded ID and ownership labels. No resources were removed. The ownership receipt and files are retained; review the previous image before retrying cleanup."
            }
            $presentImages += $current.reference
        }
    }
    if ($actual.Count) {
        $null = Invoke-HelmLocalNative $Tools.Kind @('delete', 'cluster', '--name', $Manifest.clusterName, '--kubeconfig', (Join-Path $Root 'tmp/cleanup-kubeconfig'))
        if (@(Get-HelmLocalNodes $Tools $Manifest.clusterName).Count) { throw 'Owned cluster is still present; receipts and keys retained.' }
    }
    if ($Manifest.schemaVersion -eq 3) {
        $Manifest.clusterRemoved = $true
        $Manifest.deletedNodes = @($Manifest.nodes); $Manifest.nodes = @()
        Write-HelmLocalJson (Join-Path $Root 'manifest.json') $Manifest
    }
    foreach ($reference in $presentImages) { $null = Invoke-HelmLocalNative $Tools.Docker @('image', 'rm', $reference) }
    if ($RemoveFiles) {
        Assert-HelmLocalBoundary $Root $Primary
        $null = Read-HelmLocalManifest (Join-Path $Root 'manifest.json') $Root $Manifest.clusterName $Primary
        Assert-HelmLocalNoChildReparse $Root
        Remove-HelmLocalPrivateAllocation $Root $Manifest
        Remove-Item -LiteralPath $Root -Recurse -Force
        Write-Output 'Owned cluster and run files removed.'
    } else { Write-Output "Owned cluster removed. Files and credentials retained at $Root." }
}
