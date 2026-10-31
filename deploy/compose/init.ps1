#requires -Version 7.2
# One-time material generation only. Docker Compose owns the service lifecycle.
[CmdletBinding()]
param(
    [ValidatePattern('^v\d+\.\d+\.\d+(-preview\.\d+)?$')][string] $ReleaseTag,
    [ValidatePattern('^[a-z][a-z0-9-]{0,30}$')][string] $Name = 'colossus',
    [ValidatePattern('^[a-zA-Z0-9][a-zA-Z0-9.-]{0,252}$')][string] $HostName = 'localhost',
    [ValidateRange(1,65535)][int] $Port = 9443,
    [ValidateRange(1,65535)][int] $ConnectorPort = 9444
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($Port -eq $ConnectorPort) { throw 'Browser and connector need distinct ports.' }
$root = Join-Path $PSScriptRoot '.local'
foreach ($path in @($root,(Join-Path $PSScriptRoot '.env'))) {
    if (Test-Path -LiteralPath $path) { throw 'Existing setup retained. Initialization never replaces credentials; use docker compose directly.' }
}
$cursor = $PSScriptRoot
while ($cursor) {
    if ((Get-Item -LiteralPath $cursor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Linked setup directories are not supported.' }
    $cursor = [IO.Path]::GetDirectoryName($cursor)
}
if (-not $ReleaseTag) {
    $chart = @('../control-plane/chart/Chart.yaml','../chart/Chart.yaml') | ForEach-Object { Join-Path $PSScriptRoot $_ } | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    if (-not $chart) { throw 'Pass -ReleaseTag when the matching release chart is unavailable.' }
    $ReleaseTag = 'v' + [regex]::Match([IO.File]::ReadAllText($chart),'(?m)^appVersion:\s*"?([^"\s]+)').Groups[1].Value
}
$template = @('../control-plane/controlled-project.ps1','../controlled-project.ps1') | ForEach-Object { Join-Path $PSScriptRoot $_ } | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $template) { throw 'The release-matched controlled Chat template is missing.' }
. $template
# Atomically claim the output. A failed/partial initialization is retained too.
$null = New-Item -ItemType Directory -Path $root -ErrorAction Stop
if ($IsWindows) {
    $acl = [Security.AccessControl.DirectorySecurity]::new()
    $acl.SetAccessRuleProtection($true,$false)
    $user = [Security.Principal.WindowsIdentity]::GetCurrent().User
    $acl.SetOwner($user)
    foreach ($sid in @($user,[Security.Principal.SecurityIdentifier]::new('S-1-5-18'))) {
        $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new($sid,'FullControl','ContainerInherit,ObjectInherit','None','Allow'))
    }
    Set-Acl -LiteralPath $root -AclObject $acl
} else { & chmod 700 -- $root; if ($LASTEXITCODE -ne 0) { throw 'Cannot protect setup directory.' } }
function Secret { [Convert]::ToHexString([Security.Cryptography.RandomNumberGenerator]::GetBytes(32)).ToLowerInvariant() }
function Write-Private([string] $Relative,[string] $Text) {
    $path = Join-Path $root $Relative
    [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($path))
    $stream = [IO.File]::Open($path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write)
    try { $bytes=[Text.UTF8Encoding]::new($false).GetBytes($Text.Replace("`r`n","`n")); $stream.Write($bytes); $stream.Flush($true) } finally { $stream.Dispose() }
    if (-not $IsWindows) { & chmod 600 -- $path; if ($LASTEXITCODE -ne 0) { throw 'Cannot protect setup file.' } }
}
function Certificate([string] $Role,[string[]] $Names) {
    $caKey=[Security.Cryptography.RSA]::Create(2048); $key=[Security.Cryptography.RSA]::Create(2048)
    $ca=$null; $leaf=$null
    try {
        $hash=[Security.Cryptography.HashAlgorithmName]::SHA256; $padding=[Security.Cryptography.RSASignaturePadding]::Pkcs1
        $request=[Security.Cryptography.X509Certificates.CertificateRequest]::new("CN=Colossus $Role CA",$caKey,$hash,$padding)
        $request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509BasicConstraintsExtension]::new($true,$true,0,$true))
        $request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509KeyUsageExtension]::new('KeyCertSign,CrlSign',$true))
        $ca=$request.CreateSelfSigned([DateTimeOffset]::UtcNow.AddMinutes(-5),[DateTimeOffset]::UtcNow.AddDays(730))
        $request=[Security.Cryptography.X509Certificates.CertificateRequest]::new("CN=$($Names[0])",$key,$hash,$padding)
        $san=[Security.Cryptography.X509Certificates.SubjectAlternativeNameBuilder]::new()
        foreach ($name in ($Names | Select-Object -Unique)) {
            $ip=$null
            if ([Net.IPAddress]::TryParse($name,[ref]$ip)) { $san.AddIpAddress($ip) } else { $san.AddDnsName($name) }
        }
        $request.CertificateExtensions.Add($san.Build())
        $request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509KeyUsageExtension]::new('DigitalSignature,KeyEncipherment',$true))
        $oids=[Security.Cryptography.OidCollection]::new(); [void]$oids.Add([Security.Cryptography.Oid]::new('1.3.6.1.5.5.7.3.1'))
        $request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension]::new($oids,$false))
        $leaf=$request.Create($ca,[DateTimeOffset]::UtcNow.AddMinutes(-5),[DateTimeOffset]::UtcNow.AddDays(365),[Security.Cryptography.RandomNumberGenerator]::GetBytes(16))
        Write-Private "material/$Role/server.pem" $leaf.ExportCertificatePem()
        Write-Private "material/$Role/server-key.pem" $key.ExportPkcs8PrivateKeyPem()
        Write-Private "material/$Role/ca.pem" $ca.ExportCertificatePem()
        if ($Role -eq 'controller') { Write-Private 'material/controller/ca-key.pem' $caKey.ExportPkcs8PrivateKeyPem() }
    } finally { if ($leaf) { $leaf.Dispose() }; if ($ca) { $ca.Dispose() }; $caKey.Dispose(); $key.Dispose() }
}
Certificate browser @($HostName,'localhost','browser')
Certificate controller @($HostName)
Certificate database @('postgres')
Certificate anchor-database @('anchor-postgres')
Certificate anchor-browser @('anchor-browser')
foreach ($role in @('database','anchor-database')) {
    $password=Secret
    Write-Private "material/$role/app-password" $password
    Write-Private "material/$role/admin-password" (Secret)
    foreach ($file in @('postgresql.conf','pg_hba.conf')) { Write-Private "material/$role/$file" ([IO.File]::ReadAllText((Join-Path $PSScriptRoot "config/$file"))) }
    $service=if ($role -eq 'database') { 'controller' } else { 'anchor' }
    $hostNameForDatabase=if ($role -eq 'database') { 'postgres' } else { 'anchor-postgres' }
    Write-Private "material/$service/database-url" "postgresql://colossus:$password@$($hostNameForDatabase):5432/colossus"
    Write-Private "material/$service/database-ca.pem" ([IO.File]::ReadAllText((Join-Path $root "material/$role/ca.pem")))
}
$anchorToken=Secret; $adminPassword=Secret
Write-Private 'material/anchor/anchor-token' $anchorToken
Write-Private 'material/anchor/storage-key' (Secret)
Write-Private 'material/controller/anchor-token' $anchorToken
Write-Private 'material/controller/anchor-ca.pem' ([IO.File]::ReadAllText((Join-Path $root 'material/anchor-browser/ca.pem')))
foreach ($key in @('auth-key','journal-key','signing-key','wrapping-key')) { Write-Private "material/controller/$key" (Secret) }
Write-Private 'material/controller/admin-password' $adminPassword
Write-Private 'material/bootstrap/admin-password' $adminPassword
Write-Private 'material/bootstrap/ca.pem' ([IO.File]::ReadAllText((Join-Path $root 'material/browser/ca.pem')))
Write-Private 'admin-login.txt' "Username: admin`nPassword: $adminPassword`n"
$config=Get-Content (Join-Path $PSScriptRoot 'config/control-plane.example.json') -Raw | ConvertFrom-Json -AsHashtable
$config.public_origin="https://$($HostName):$Port"; $config.grpc_endpoint="https://$($HostName):$ConnectorPort"
$config.chat.projects.local=New-ColossusControlledProject
$config.chat.projects.local.runtime.storage.path='/var/lib/colossus/chat/template.redb'
$config.chat.projects.local.runtime.storage.keys.anchor_path='/var/lib/colossus/chat/template.anchor'
Write-Private 'material/controller/config.json' ($config | ConvertTo-Json -Depth 40)
Write-Private 'material/anchor/config.json' ([IO.File]::ReadAllText((Join-Path $PSScriptRoot 'config/anchor.example.json')))
$environment=[IO.File]::ReadAllText((Join-Path $PSScriptRoot '.env.example')).Replace('v0.11.11-preview.5',$ReleaseTag).Replace('COMPOSE_PROJECT_NAME=colossus',"COMPOSE_PROJECT_NAME=$Name").Replace('=9443',"=$Port").Replace('=9444',"=$ConnectorPort")
$environment=$environment.Replace('COLOSSUS_PUBLIC_ORIGIN=https://localhost:9443',"COLOSSUS_PUBLIC_ORIGIN=https://$($HostName):$Port")
$file=[IO.File]::Open((Join-Path $PSScriptRoot '.env'),[IO.FileMode]::CreateNew)
try { $bytes=[Text.UTF8Encoding]::new($false).GetBytes($environment); $file.Write($bytes) } finally { $file.Dispose() }
Write-Output "Initialized. Run docker compose up -d --wait, then open https://$($HostName):$Port."
Write-Output "Login: $root/admin-login.txt; browser CA: $root/material/browser/ca.pem"
