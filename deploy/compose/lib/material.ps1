function New-ComposeSecret {
    return [Convert]::ToHexString([Security.Cryptography.RandomNumberGenerator]::GetBytes(32)).ToLowerInvariant()
}

function New-ComposeTls {
    param([string[]] $Names)
    $caKey = [Security.Cryptography.RSA]::Create(2048)
    $key = [Security.Cryptography.RSA]::Create(2048)
    $ca = $null; $leaf = $null
    try {
        $hash = [Security.Cryptography.HashAlgorithmName]::SHA256
        $padding = [Security.Cryptography.RSASignaturePadding]::Pkcs1
        $request = [Security.Cryptography.X509Certificates.CertificateRequest]::new('CN=Colossus Compose private CA', $caKey, $hash, $padding)
        [void]$request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509BasicConstraintsExtension]::new($true, $true, 0, $true))
        [void]$request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509KeyUsageExtension]::new('KeyCertSign,CrlSign', $true))
        $ca = $request.CreateSelfSigned([DateTimeOffset]::UtcNow.AddMinutes(-5), [DateTimeOffset]::UtcNow.AddDays(730))
        $request = [Security.Cryptography.X509Certificates.CertificateRequest]::new("CN=$($Names[0])", $key, $hash, $padding)
        [void]$request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509BasicConstraintsExtension]::new($false, $false, 0, $true))
        [void]$request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509KeyUsageExtension]::new('DigitalSignature,KeyEncipherment', $true))
        $san = [Security.Cryptography.X509Certificates.SubjectAlternativeNameBuilder]::new()
        foreach ($name in ($Names | Select-Object -Unique)) {
            $address = $null
            if ([Net.IPAddress]::TryParse($name, [ref]$address)) { $san.AddIpAddress($address) } else { $san.AddDnsName($name) }
        }
        [void]$request.CertificateExtensions.Add($san.Build())
        $oids = [Security.Cryptography.OidCollection]::new()
        [void]$oids.Add([Security.Cryptography.Oid]::new('1.3.6.1.5.5.7.3.1'))
        [void]$request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension]::new($oids, $false))
        $leaf = $request.Create($ca, [DateTimeOffset]::UtcNow.AddMinutes(-5), [DateTimeOffset]::UtcNow.AddDays(365), [Security.Cryptography.RandomNumberGenerator]::GetBytes(16))
        return @{ ca = $ca.ExportCertificatePem(); caKey = $caKey.ExportPkcs8PrivateKeyPem(); certificate = $leaf.ExportCertificatePem(); key = $key.ExportPkcs8PrivateKeyPem() }
    } finally {
        if ($null -ne $leaf) { $leaf.Dispose() }; if ($null -ne $ca) { $ca.Dispose() }
        $caKey.Dispose(); $key.Dispose()
    }
}

function Write-ComposeRole {
    param([string] $Root, [string] $Role, [hashtable] $Files)
    $directory = Join-Path $Root "material/$Role"
    [void][IO.Directory]::CreateDirectory($directory)
    Protect-ComposeDirectory $directory
    foreach ($entry in $Files.GetEnumerator()) { Write-ComposeFile (Join-Path $directory $entry.Key) ([string]$entry.Value) }
    $lines = @(Get-ChildItem -LiteralPath $directory -File | Sort-Object Name | ForEach-Object { (Get-ComposeHash $_.FullName) + '  ' + $_.Name })
    Write-ComposeFile (Join-Path $directory 'SHA256SUMS') (($lines -join "`n") + "`n")
}

function New-ComposeDatabaseMaterial {
    param([string] $Root, [string] $Role, [hashtable] $Tls, [string] $Password)
    Write-ComposeRole $Root $Role @{
        'admin-password' = New-ComposeSecret
        'app-password' = $Password
        'server.pem' = $Tls.certificate
        'server-key.pem' = $Tls.key
        'postgresql.conf' = @'
listen_addresses='*'
ssl=on
ssl_cert_file='/run/colossus/server.pem'
ssl_key_file='/run/colossus/server-key.pem'
ssl_min_protocol_version='TLSv1.2'
password_encryption='scram-sha-256'
hba_file='/run/colossus/pg_hba.conf'
shared_buffers='64MB'
log_statement='none'
log_min_error_statement='panic'
'@
        'pg_hba.conf' = @'
local all all trust
hostnossl all all 0.0.0.0/0 reject
hostnossl all all ::/0 reject
hostssl colossus colossus 0.0.0.0/0 scram-sha-256
hostssl colossus colossus ::/0 scram-sha-256
host all all 0.0.0.0/0 reject
host all all ::/0 reject
'@
    }
}

function Initialize-ComposeMaterial {
    param([string] $Root, [hashtable] $Manifest, [string] $Bundle)
    $browser = New-ComposeTls @($Manifest.hostname, 'localhost')
    $connector = New-ComposeTls @($Manifest.hostname)
    $database = New-ComposeTls @('postgres')
    $password = New-ComposeSecret
    $adminPassword = New-ComposeSecret
    New-ComposeDatabaseMaterial $Root 'database' $database $password
    Write-ComposeRole $Root 'browser' @{ 'ca.pem' = $browser.ca; 'server.pem' = $browser.certificate; 'server-key.pem' = $browser.key }
    $databaseConfig = @{ connectionVariable = 'COLOSSUS_CONTROL_PLANE_DATABASE'; schema = 'colossus_cloud'; tls = @{ kind = 'custom_ca'; caPemPath = '/run/colossus/database-ca.pem' }; maxConnections = 8; connectionTimeoutMs = 5000; statementTimeoutMs = 15000 }
    $config = @{
        http_bind = '0.0.0.0:8080'; grpc_bind = '0.0.0.0:8443'
        public_origin = "https://$($Manifest.hostname):$($Manifest.browserPort)"
        grpc_endpoint = "https://$($Manifest.hostname):$($Manifest.connectorPort)"
        ca_certificate = '/run/colossus/ca.pem'; ca_key = '/run/colossus/ca-key.pem'
        server_certificate = '/run/colossus/server.pem'; server_key = '/run/colossus/server-key.pem'
        web_root = '/opt/colossus/web'; local_development = $false; memberships = @()
        database = $databaseConfig; auth_key_variable = 'COLOSSUS_CONTROL_PLANE_AUTH_KEY'
        local_auth = @{ session_seconds = 28800 }; oidc = $null
        bootstrap_admin = @{ display_name = 'Administrator'; username = 'admin'; password_variable = 'COLOSSUS_CONTROL_PLANE_BOOTSTRAP_PASSWORD' }
    }
    $files = @{
        'database-url' = "postgresql://colossus:$password@postgres:5432/colossus"
        'database-ca.pem' = $database.ca
        'ca.pem' = $connector.ca; 'ca-key.pem' = $connector.caKey
        'server.pem' = $connector.certificate; 'server-key.pem' = $connector.key
        'auth-key' = New-ComposeSecret; 'admin-password' = $adminPassword
    }
    if ($Manifest.profile -ceq 'hosted-demo') {
        $anchorDatabase = New-ComposeTls @('anchor-postgres')
        $anchorBrowser = New-ComposeTls @('anchor-browser')
        $anchorPassword = New-ComposeSecret
        $anchorToken = New-ComposeSecret
        New-ComposeDatabaseMaterial $Root 'anchor-database' $anchorDatabase $anchorPassword
        Write-ComposeRole $Root 'anchor-browser' @{ 'ca.pem' = $anchorBrowser.ca; 'server.pem' = $anchorBrowser.certificate; 'server-key.pem' = $anchorBrowser.key }
        $anchorConfig = @{
            bind = '0.0.0.0:8080'; trustedTlsIngress = $true
            database = @{ connectionVariable = 'COLOSSUS_ANCHOR_DATABASE'; schema = 'colossus_chat_anchors'; tls = @{ kind = 'custom_ca'; caPemPath = '/run/colossus/database-ca.pem' }; maxConnections = 4; connectionTimeoutMs = 5000; statementTimeoutMs = 5000 }
            bearerVariable = 'COLOSSUS_ANCHOR_TOKEN'; storageKeyVariable = 'COLOSSUS_ANCHOR_STORAGE_KEY'
        }
        Write-ComposeRole $Root 'anchor' @{
            'config.json' = $anchorConfig | ConvertTo-Json -Depth 12
            'database-url' = "postgresql://colossus:$anchorPassword@anchor-postgres:5432/colossus"
            'database-ca.pem' = $anchorDatabase.ca; 'anchor-token' = $anchorToken; 'storage-key' = New-ComposeSecret
        }
        # Reuse the same reviewed template as the Helm adapter.
        $projectFile = @((Join-Path $Bundle '../control-plane/controlled-project.ps1'), (Join-Path $Bundle '../controlled-project.ps1')) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
        if (-not $projectFile) { throw 'The release-matched controlled Hosted template is missing.' }
        . $projectFile
        $project = New-ColossusControlledProject
        $project.runtime.storage.path = '/var/lib/colossus/chat/template.redb'
        $project.runtime.storage.keys.anchor_path = '/var/lib/colossus/chat/template.anchor'
        $config.chat = @{
            home = '/var/lib/colossus/chat'; wrapping_key_variable = 'COLOSSUS_CHAT_WRAPPING_KEY'
            max_active_runtimes = 2; idle_seconds = 300; projects = @{ local = $project }
            shared = @{ keys = @{ journal_variable = 'COLOSSUS_CHAT_JOURNAL_KEY'; signing_variable = 'COLOSSUS_CHAT_SIGNING_KEY'; anchor_endpoint = 'https://anchor-browser:8443/v1/anchors'; anchor_bearer_variable = 'COLOSSUS_CHAT_ANCHOR_TOKEN'; allow_loopback_http = $false; anchor_ca_pem_path = '/run/colossus/anchor-ca.pem' } }
        }
        $files['anchor-ca.pem'] = $anchorBrowser.ca
        $files['anchor-token'] = $anchorToken
        foreach ($key in @('journal-key','signing-key','wrapping-key')) { $files[$key] = New-ComposeSecret }
    }
    $files['config.json'] = $config | ConvertTo-Json -Depth 40
    Write-ComposeRole $Root 'controller' $files
    # Only public certificates are exported outside service-specific material.
    Write-ComposeFile (Join-Path $Root 'browser-ca.pem') $browser.ca
    Write-ComposeFile (Join-Path $Root 'connector-ca.pem') $connector.ca
    Write-ComposeFile (Join-Path $Root 'admin-login.txt') ("Username: admin`nPassword: $adminPassword`n")
}

function Get-ComposeEnvironmentText {
    param($Manifest)
    $profile = if ($Manifest.profile -ceq 'hosted-demo') { 'hosted' } else { '' }
    return @"
COMPOSE_PROJECT_NAME=$($Manifest.project)
COMPOSE_PROFILES=$profile
COLOSSUS_DEPLOYMENT_ID=$($Manifest.deploymentId)
COLOSSUS_SERVER_IMAGE=$($Manifest.serverImage)
COLOSSUS_DOCUMENTATION_IMAGE=$($Manifest.documentationImage)
COLOSSUS_BIND_ADDRESS=$($Manifest.bindAddress)
COLOSSUS_BROWSER_PORT=$($Manifest.browserPort)
COLOSSUS_CONNECTOR_PORT=$($Manifest.connectorPort)
"@
}

function Initialize-ComposeDeployment {
    param([string] $Root, [string] $Bundle, [string] $Name, [string] $ReleaseTag, [string] $HostName, [string] $BindAddress, [int] $Port, [int] $ConnectorPort, [switch] $CoreOnly, [switch] $LocalImages)
    if ($Port -eq $ConnectorPort) { throw 'Browser and connector need distinct ports.' }
    if ($HostName -cnotmatch '^[a-zA-Z0-9][a-zA-Z0-9.-]{0,252}$') { throw 'Use a DNS hostname or IPv4 address without a port.' }
    $address = $null
    if (-not [Net.IPAddress]::TryParse($BindAddress, [ref]$address) -or $address.AddressFamily -ne [Net.Sockets.AddressFamily]::InterNetwork) { throw 'BindAddress must be a literal IPv4 address.' }
    if ($ReleaseTag -cnotmatch '^v\d+\.\d+\.\d+(-preview\.\d+)?$') { throw 'Select one exact stable or preview release tag.' }
    if (Test-Path -LiteralPath $Root) { throw 'Deployment path already exists; retained state will not be adopted or replaced.' }
    $parent = [IO.Path]::GetDirectoryName($Root)
    Assert-ComposePath $parent
    if (-not (Test-Path -LiteralPath $parent)) { [void][IO.Directory]::CreateDirectory($parent); Protect-ComposeDirectory $parent }
    Assert-ComposePrivateDirectory $parent
    $null = New-Item -ItemType Directory -Path $Root -ErrorAction Stop; Protect-ComposeDirectory $Root
    $id = [Guid]::NewGuid().ToString('N')
    $manifest = @{
        schemaVersion = 1; purpose = 'ColossusCompose'; deploymentId = $id; owner = Get-ComposeOwner
        name = $Name; project = "colossus-$Name-$($id.Substring(0,8))"; root = $Root
        hostname = $HostName; bindAddress = $BindAddress; browserPort = $Port; connectorPort = $ConnectorPort
        profile = $(if ($CoreOnly) { 'core' } else { 'hosted-demo' }); releaseTag = $ReleaseTag
        serverImage = "ghcr.io/obscuritylabs/colossus-control-plane:$ReleaseTag"
        documentationImage = "ghcr.io/obscuritylabs/colossus-documentation:$ReleaseTag"
        localImages = $LocalImages.IsPresent
        provisioned = $false; dataRemoved = $false; engineId = ''; files = @{}
        createdAtUtc = [DateTime]::UtcNow.ToString('o')
    }
    # Record allocation intent before generating any credentials. A partial first
    # initialization remains inspectable; reruns never replace its missing keys.
    Write-ComposeJson (Join-Path $Root 'manifest.json') $manifest
    [void][IO.Directory]::CreateDirectory((Join-Path $Root 'runtime'))
    Protect-ComposeDirectory (Join-Path $Root 'runtime')
    foreach ($item in Get-ChildItem -LiteralPath (Join-Path $Bundle 'runtime') -File) { Write-ComposeFile (Join-Path $Root "runtime/$($item.Name)") ([IO.File]::ReadAllText($item.FullName)) }
    Write-ComposeFile (Join-Path $Root 'compose.yaml') ([IO.File]::ReadAllText((Join-Path $Bundle 'compose.yaml')))
    Initialize-ComposeMaterial $Root $manifest $Bundle
    Write-ComposeFile (Join-Path $Root '.env') (Get-ComposeEnvironmentText $manifest)
    foreach ($file in Get-ChildItem -LiteralPath $Root -File -Recurse -Force | Where-Object Name -ne 'manifest.json') {
        $relative = [IO.Path]::GetRelativePath($Root, $file.FullName).Replace('\','/')
        $manifest.files[$relative] = Get-ComposeHash $file.FullName
    }
    $manifest['initialized'] = $true
    Write-ComposeJson (Join-Path $Root 'manifest.json') $manifest
    return $manifest
}
