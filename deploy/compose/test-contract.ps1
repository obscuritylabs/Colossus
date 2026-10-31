#requires -Version 7.2
# Offline boundary/PKI/recovery contracts. No Docker engine or model is contacted.
[CmdletBinding()]
param([Parameter(Mandatory)][string] $ScratchDirectory)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'lib/private.ps1')
. (Join-Path $PSScriptRoot 'lib/material.ps1')
. (Join-Path $PSScriptRoot 'lib/docker.ps1')
. (Join-Path $PSScriptRoot 'lib/backup.ps1')
. (Join-Path $PSScriptRoot 'lib/bootstrap.ps1')
$id=[Guid]::NewGuid().ToString('N')
$fixture=Join-Path ([IO.Path]::GetFullPath($ScratchDirectory)) "compose-contract-$id"
Assert-ComposePath $fixture
$null=New-Item -ItemType Directory -Path $fixture -ErrorAction Stop
Protect-ComposeDirectory $fixture
Write-ComposeJson (Join-Path $fixture 'ownership.json') @{purpose='ColossusComposeContract';id=$id;root=$fixture}
function Assert-That { param([bool]$Condition,[string]$Message); if(-not $Condition){throw $Message} }
function Assert-Rejected {
    param([scriptblock]$Operation,[string]$Pattern)
    try { & $Operation | Out-Null } catch {
        if($_.Exception.Message -notmatch $Pattern){throw "Unexpected rejection: $($_.Exception.Message)"}
        return
    }
    throw "Operation was accepted; expected $Pattern"
}
$passed=$false
try {
    $root=Get-ComposeRoot 'default' $fixture
    $manifest=Initialize-ComposeDeployment $root $PSScriptRoot 'default' 'v0.11.11-preview.5' 'localhost' '127.0.0.1' 19443 19444
    Assert-ComposeManifest $root $manifest
    Assert-ComposeCertificates $root
    $client=New-ComposeHttpClient (Join-Path $root 'browser-ca.pem')
    $client.Dispose()
    Assert-That ($manifest.profile -ceq 'hosted-demo') 'Chat must be enabled by default.'
    $config=Get-Content -LiteralPath (Join-Path $root 'material/controller/config.json') -Raw | ConvertFrom-Json -AsHashtable
    Assert-That (-not $config.local_development -and $config.database.tls.kind -ceq 'custom_ca') 'Compose must retain production transport validation.'
    Assert-That ($config.chat.shared.keys.anchor_endpoint -ceq 'https://anchor-browser:8443/v1/anchors') 'Hosted anchors must use verified HTTPS.'
    Assert-That (-not (Test-Path -LiteralPath (Join-Path $root 'material/controller/storage-key'))) 'Anchor storage key crossed its service boundary.'
    $controllerPassword=[IO.File]::ReadAllText((Join-Path $root 'material/database/app-password'))
    $anchorPassword=[IO.File]::ReadAllText((Join-Path $root 'material/anchor-database/app-password'))
    Assert-That ($controllerPassword -cne $anchorPassword) 'The two databases share a password.'
    $environment=[IO.File]::ReadAllText((Join-Path $root '.env'))
    Assert-That (-not $environment.Contains($controllerPassword) -and -not $environment.Contains($anchorPassword)) 'Secrets leaked into Compose interpolation.'
    $hash=Get-ComposeHash (Join-Path $root 'material/controller/auth-key')
    Assert-Rejected { Initialize-ComposeDeployment $root $PSScriptRoot 'default' 'v0.11.11-preview.5' 'localhost' '127.0.0.1' 19443 19444 } 'already exists'
    Assert-That ((Get-ComposeHash (Join-Path $root 'material/controller/auth-key')) -ceq $hash) 'Reinitialization changed a retained key.'
    $coreRoot=Get-ComposeRoot 'core' $fixture
    $core=Initialize-ComposeDeployment $coreRoot $PSScriptRoot 'core' 'v0.11.11-preview.5' 'localhost' '127.0.0.1' 19445 19446 -CoreOnly
    Assert-That ($core.profile -ceq 'core' -and -not (Test-Path -LiteralPath (Join-Path $coreRoot 'material/anchor'))) 'Core-only mode allocated Hosted authority.'
    $preview=Get-ComposeRoot 'preview' $fixture
    & (Join-Path $PSScriptRoot 'compose.ps1') -Action init -Name preview -StateDirectory $fixture -Docker missing-docker -WhatIf
    Assert-That (-not (Test-Path -LiteralPath $preview)) 'WhatIf allocated deployment state.'
    $old=$manifest.project; $manifest.project='colossus-foreign-12345678'
    Assert-Rejected { Assert-ComposeManifest $root $manifest } 'ownership receipt'
    $manifest.project=$old
    $manifest.provisioned=$true; $manifest['volumeNames']=@('retained-database')
    Assert-Rejected { Assert-ComposeRetainedVolumes $manifest @{volumes=@()} } 'volume is missing'
    $manifest.provisioned=$false
    Assert-ComposeUpgrade 'v0.11.11-preview.4' 'v0.11.11-preview.5'
    Assert-ComposeUpgrade 'v0.11.11-preview.5' 'v0.11.11'
    Assert-Rejected { Assert-ComposeUpgrade 'v0.11.11-preview.5' 'v0.11.11-preview.4' } 'newer release'
    Assert-Rejected { Assert-ComposeUpgrade 'v0.11.11' 'v0.11.11-preview.6' } 'newer release'
    $manifest['restoring']=$true
    Write-ComposeJson (Join-Path $root 'manifest.json') $manifest
    Assert-Rejected { & (Join-Path $PSScriptRoot 'compose.ps1') -Action up -Name default -StateDirectory $fixture -Docker missing-docker } 'Restore was interrupted'
    $manifest.restoring=$false
    Write-ComposeJson (Join-Path $root 'manifest.json') $manifest
    $expired=Join-Path $coreRoot 'material/browser/server.pem'
    $key=[Security.Cryptography.RSA]::Create(2048)
    $cert=$null
    try {
        $request=[Security.Cryptography.X509Certificates.CertificateRequest]::new('CN=expired', $key, [Security.Cryptography.HashAlgorithmName]::SHA256, [Security.Cryptography.RSASignaturePadding]::Pkcs1)
        $cert=$request.CreateSelfSigned([DateTimeOffset]::UtcNow.AddDays(-2),[DateTimeOffset]::UtcNow.AddDays(-1))
        [IO.File]::WriteAllText($expired,$cert.ExportCertificatePem())
    } finally { if($null -ne $cert){$cert.Dispose()}; $key.Dispose() }
    $core.files['material/browser/server.pem']=Get-ComposeHash $expired
    Write-ComposeJson (Join-Path $coreRoot 'manifest.json') $core
    Assert-ComposeManifest $coreRoot $core
    Assert-Rejected { Assert-ComposeCertificates $coreRoot } 'requires deliberate renewal'
    & (Join-Path $PSScriptRoot 'compose.ps1') -Action down -Name core -StateDirectory $fixture -Docker missing-docker -WhatIf
    $originalInvoke=(Get-Item Function:Invoke-ComposeDocker).ScriptBlock
    try {
        function Invoke-ComposeDocker { param($Docker,$Arguments); return 'fixture' }
        $env:COLOSSUS_BROWSER_PORT='ambient-wrong-port'
        Remove-Item Env:COLOSSUS_DEPLOYMENT_ID -ErrorAction SilentlyContinue
        $null=Invoke-Compose missing $root $manifest @('config','--quiet')
        Assert-That ($env:COLOSSUS_BROWSER_PORT -ceq 'ambient-wrong-port') 'Compose did not restore the caller environment.'
        Assert-That (-not (Test-Path Env:COLOSSUS_DEPLOYMENT_ID)) 'Compose leaked temporary environment selection.'
        Remove-Item Env:COLOSSUS_BROWSER_PORT
    } finally { Set-Item Function:Invoke-ComposeDocker $originalInvoke }
    $certificate=Join-Path $coreRoot 'material/controller/server.pem'
    Remove-Item -LiteralPath $certificate
    Assert-Rejected { Assert-ComposeManifest $coreRoot $core } 'will not be regenerated'
    $linked=Join-Path $fixture 'linked'
    if($IsWindows){$null=New-Item -ItemType Junction -Path $linked -Target $root}else{$null=New-Item -ItemType SymbolicLink -Path $linked -Target $root}
    Assert-Rejected { Get-ComposeRoot 'escape' $linked } 'Linked deployment paths'
    Remove-Item -LiteralPath $linked
    $archive=Join-Path $fixture 'malicious.zip'
    $zip=[IO.Compression.ZipFile]::Open($archive,[IO.Compression.ZipArchiveMode]::Create)
    try {
        $entry=$zip.CreateEntry('../outside.txt');$writer=[IO.StreamWriter]::new($entry.Open());$writer.Write('must not escape');$writer.Dispose()
    } finally {$zip.Dispose()}
    Write-ComposeFile ($archive+'.sha256') ((Get-ComposeHash $archive)+'  malicious.zip'+"`n")
    Assert-Rejected {
        $restoreLock=$null
        try { Restore-ComposeBackup missing (Get-ComposeRoot 'restore' $fixture) 'restore' $archive ([ref]$restoreLock) }
        finally { if($null -ne $restoreLock){$restoreLock.Dispose()} }
    } 'Unsafe archive entry'
    Assert-That (-not (Test-Path -LiteralPath (Join-Path $fixture 'outside.txt'))) 'Archive extraction escaped its allocation.'
    $passed=$true
    Write-Output 'Compose offline contracts passed: chat default, TLS, key custody, retained state, ownership, environment, WhatIf, link rejection, and archive confinement.'
} finally {
    if($passed){
        $owner=Get-Content -LiteralPath (Join-Path $fixture 'ownership.json') -Raw | ConvertFrom-Json -AsHashtable
        Assert-That ($owner.id -ceq $id -and $owner.root -ceq $fixture) 'Contract cleanup identity changed.'
        Assert-ComposePath $fixture
        foreach($item in Get-ChildItem -LiteralPath $fixture -Recurse -Force){if($item.Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'Contract cleanup found a link.'}}
        if([IO.Path]::GetDirectoryName($fixture) -cne [IO.Path]::GetFullPath($ScratchDirectory) -or [IO.Path]::GetFileName($fixture) -cne "compose-contract-$id"){throw 'Contract cleanup boundary changed.'}
        Remove-Item -LiteralPath $fixture -Recurse -Force
    }else{Write-Output "Contract evidence retained at $fixture"}
}
