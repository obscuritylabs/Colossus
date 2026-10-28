# Private Hosted material is retained outside source checkouts. Never print keys,
# passwords, Secret bodies or native output while provisioning these resources.
function Get-HelmLocalSha256 {
    param([byte[]] $Bytes)
    return [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($Bytes)).ToLowerInvariant()
}

function Write-HelmLocalPrivateFile {
    param([string] $Path, [string] $Text)
    $stream = [IO.File]::Open($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes($Text)
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
    } finally { $stream.Dispose() }
}

function New-HelmLocalTlsMaterial {
    param([string] $Directory, [string] $Prefix, [string[]] $DnsNames)
    $caKey = [Security.Cryptography.RSA]::Create(2048)
    $leafKey = [Security.Cryptography.RSA]::Create(2048)
    $ca = $null; $leaf = $null
    try {
        $hash = [Security.Cryptography.HashAlgorithmName]::SHA256
        $padding = [Security.Cryptography.RSASignaturePadding]::Pkcs1
        $caRequest = [Security.Cryptography.X509Certificates.CertificateRequest]::new("CN=Colossus local $Prefix CA", $caKey, $hash, $padding)
        [void]$caRequest.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509BasicConstraintsExtension]::new($true, $true, 0, $true))
        $caUsage = [Security.Cryptography.X509Certificates.X509KeyUsageFlags]::KeyCertSign -bor [Security.Cryptography.X509Certificates.X509KeyUsageFlags]::CrlSign
        [void]$caRequest.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509KeyUsageExtension]::new($caUsage, $true))
        $ca = $caRequest.CreateSelfSigned([DateTimeOffset]::UtcNow.AddMinutes(-5), [DateTimeOffset]::UtcNow.AddDays(365))
        $request = [Security.Cryptography.X509Certificates.CertificateRequest]::new("CN=$($DnsNames[0])", $leafKey, $hash, $padding)
        [void]$request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509BasicConstraintsExtension]::new($false, $false, 0, $true))
        $san = [Security.Cryptography.X509Certificates.SubjectAlternativeNameBuilder]::new()
        foreach ($name in $DnsNames) { $san.AddDnsName($name) }
        [void]$request.CertificateExtensions.Add($san.Build())
        $oids = [Security.Cryptography.OidCollection]::new()
        [void]$oids.Add([Security.Cryptography.Oid]::new('1.3.6.1.5.5.7.3.1'))
        [void]$request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension]::new($oids, $false))
        $usage = [Security.Cryptography.X509Certificates.X509KeyUsageFlags]::DigitalSignature -bor [Security.Cryptography.X509Certificates.X509KeyUsageFlags]::KeyEncipherment
        [void]$request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509KeyUsageExtension]::new($usage, $true))
        $serial = [Security.Cryptography.RandomNumberGenerator]::GetBytes(16)
        $leaf = $request.Create($ca, [DateTimeOffset]::UtcNow.AddMinutes(-5), [DateTimeOffset]::UtcNow.AddDays(90), $serial)
        Write-HelmLocalPrivateFile (Join-Path $Directory "$Prefix-ca.pem") $ca.ExportCertificatePem()
        Write-HelmLocalPrivateFile (Join-Path $Directory "$Prefix-ca-key.pem") $caKey.ExportPkcs8PrivateKeyPem()
        Write-HelmLocalPrivateFile (Join-Path $Directory "$Prefix-server.pem") $leaf.ExportCertificatePem()
        Write-HelmLocalPrivateFile (Join-Path $Directory "$Prefix-server-key.pem") $leafKey.ExportPkcs8PrivateKeyPem()
    } finally {
        if ($null -ne $leaf) { $leaf.Dispose() }
        if ($null -ne $ca) { $ca.Dispose() }
        $leafKey.Dispose(); $caKey.Dispose()
    }
}

function Initialize-HelmLocalHostedMaterial {
    param([string] $Root, [string] $Cluster, [hashtable] $Manifest)
    if (-not $Manifest.hosted.privateAllocation.path) { throw 'Allocate the protected Hosted directory before generating material.' }
    $allocation = [IO.Path]::GetFullPath([string]$Manifest.hosted.privateAllocation.path)
    Assert-HelmLocalPrivateAcl $allocation
    Assert-HelmLocalNoChildReparse $allocation
    $directory = Join-Path $allocation 'hosted-material'
    $receiptPath = Join-Path $directory 'material.json'
    if ($Manifest.hosted.ContainsKey('material') -and -not (Test-Path -LiteralPath $directory -PathType Container)) {
        throw 'Retained Hosted material directory is missing; refusing replacement keys.'
    }
    $names = @{
        controller = @("colossus-controller-postgres.$Cluster-data.svc", "colossus-controller-postgres.$Cluster-data.svc.cluster.local")
        anchorDatabase = @("colossus-anchor-postgres.$Cluster-anchors.svc", "colossus-anchor-postgres.$Cluster-anchors.svc.cluster.local")
        anchorHttps = @("colossus-chat-anchor.$Cluster-anchors.svc", "colossus-chat-anchor.$Cluster-anchors.svc.cluster.local")
    }
    if (Test-Path -LiteralPath $directory) {
        if (-not (Test-Path -LiteralPath $receiptPath -PathType Leaf)) { throw 'Incomplete Hosted material exists; preserve it and inspect the owned allocation. Keys will not be regenerated.' }
        if ($Manifest.hosted.ContainsKey('material') -and
            (Get-HelmLocalSha256 ([IO.File]::ReadAllBytes($receiptPath))) -cne $Manifest.hosted.material.receiptSha256) {
            throw 'Retained Hosted material receipt changed; refusing key rotation.'
        }
        $receipt = Get-Content -LiteralPath $receiptPath -Raw | ConvertFrom-Json -AsHashtable
        if ($receipt.cluster -cne $Cluster -or $receipt.ownerId -cne $Manifest.hosted.ownerId) { throw 'Hosted material belongs to another deployment.' }
        foreach ($file in $receipt.files) {
            if ([IO.Path]::GetFileName([string]$file.name) -cne $file.name) { throw 'Invalid retained material filename.' }
            $path = Join-Path $directory $file.name
            if (-not (Test-Path -LiteralPath $path -PathType Leaf) -or (Get-HelmLocalSha256 ([IO.File]::ReadAllBytes($path))) -cne $file.sha256) { throw 'Retained Hosted material changed or is missing; refusing key rotation.' }
        }
        foreach ($prefix in $names.Keys) {
            $cert = [Security.Cryptography.X509Certificates.X509Certificate2]::CreateFromPemFile((Join-Path $directory "$prefix-server.pem"), (Join-Path $directory "$prefix-server-key.pem"))
            try {
                if ($cert.NotAfter.ToUniversalTime() -lt [DateTime]::UtcNow.AddDays(1)) { throw 'Retained local TLS certificate needs deliberate renewal.' }
            } finally { $cert.Dispose() }
        }
    } else {
        [void][IO.Directory]::CreateDirectory($directory)
        foreach ($prefix in $names.Keys) { New-HelmLocalTlsMaterial $directory $prefix $names[$prefix] }
        foreach ($name in @('controller-admin-password', 'controller-app-password', 'anchor-admin-password', 'anchor-app-password', 'anchor-token', 'anchor-storage-key', 'journal-key', 'signing-key', 'wrapping-key')) {
            Write-HelmLocalPrivateFile (Join-Path $directory $name) ([Convert]::ToHexString([Security.Cryptography.RandomNumberGenerator]::GetBytes(32)).ToLowerInvariant())
        }
        $files = @(Get-ChildItem -LiteralPath $directory -File | Sort-Object Name | ForEach-Object { @{ name = $_.Name; sha256 = Get-HelmLocalSha256 ([IO.File]::ReadAllBytes($_.FullName)) } })
        $receipt = @{ schemaVersion = 1; cluster = $Cluster; ownerId = $Manifest.hosted.ownerId; files = $files; createdAtUtc = [DateTime]::UtcNow.ToString('o') }
        Write-HelmLocalPrivateFile $receiptPath (($receipt | ConvertTo-Json -Depth 8) + "`n")
    }
    $Manifest.hosted.material = @{ directory = $directory; receipt = $receiptPath; receiptSha256 = Get-HelmLocalSha256 ([IO.File]::ReadAllBytes($receiptPath)) }
    Write-HelmLocalJson (Join-Path $Root 'manifest.json') $Manifest
    return $directory
}

function New-HelmLocalSecret {
    param([string] $Namespace, [string] $Name, [hashtable] $Values, [string] $Type = 'Opaque')
    $data = @{}
    foreach ($key in $Values.Keys) { $data[$key] = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes([string]$Values[$key])) }
    return @{ apiVersion = 'v1'; kind = 'Secret'; metadata = @{ namespace = $Namespace; name = $Name }; type = $Type; data = $data }
}

function Get-HelmLocalMaterialText {
    param([string] $Directory, [string] $Name)
    return [IO.File]::ReadAllText((Join-Path $Directory $Name))
}
