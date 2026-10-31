#requires -Version 7.2
param([Parameter(Mandatory)][string] $ScratchDirectory)
$ErrorActionPreference='Stop'
$fixture=Join-Path ([IO.Path]::GetFullPath($ScratchDirectory)) ('compose-init-'+[Guid]::NewGuid().ToString('N'))
$passed=$false
try {
    $null=New-Item -ItemType Directory -Path (Join-Path $fixture 'deploy/control-plane')
    $compose=Join-Path $fixture 'deploy/compose'
    $null=New-Item -ItemType Directory -Path $compose
    Get-ChildItem -LiteralPath $PSScriptRoot -Force | Where-Object Name -NotIn @('.env','.local') | Copy-Item -Destination $compose -Recurse
    $template=@('../control-plane/controlled-project.ps1','../controlled-project.ps1') | ForEach-Object {Join-Path $PSScriptRoot $_} | Where-Object {Test-Path -LiteralPath $_} | Select-Object -First 1
    Copy-Item -LiteralPath $template -Destination (Join-Path $fixture 'deploy/control-plane/controlled-project.ps1')
    & (Join-Path $compose 'init.ps1') -ReleaseTag v0.11.11-preview.5
    $config=Get-Content (Join-Path $compose '.local/material/controller/config.json') -Raw | ConvertFrom-Json
    if($config.local_development -or $config.database.tls.kind -ne 'custom_ca' -or $config.chat.projects.local.runtime.providers.profiles.main.kind -ne 'echo'){throw 'Chat default or verified TLS changed'}
    $key=Join-Path $compose '.local/material/controller/auth-key'
    $before=(Get-FileHash -LiteralPath $key).Hash
    $rejected=$false
    try { & (Join-Path $compose 'init.ps1') -ReleaseTag v0.11.11-preview.5 } catch { $rejected=$_.Exception.Message -match 'Existing setup retained' }
    if(-not $rejected -or (Get-FileHash -LiteralPath $key).Hash -cne $before){throw 'Initialization replaced retained keys'}
    $db=[IO.File]::ReadAllText((Join-Path $compose '.local/material/database/app-password'))
    $anchor=[IO.File]::ReadAllText((Join-Path $compose '.local/material/anchor-database/app-password'))
    $environment=[IO.File]::ReadAllText((Join-Path $compose '.env'))
    if($db -eq $anchor -or $environment.Contains($db) -or $environment.Contains($anchor)){throw 'Database secrets crossed their boundary'}
    if(Test-Path (Join-Path $compose '.local/material/controller/storage-key')){throw 'Anchor storage key crossed its boundary'}
    if($IsWindows){if(-not (Get-Acl (Join-Path $compose '.local')).AreAccessRulesProtected){throw 'Private setup ACL is inherited'}}
    elseif((& stat -c '%a' (Join-Path $compose '.local')).Trim() -ne '700'){throw 'Setup directory is not private'}
    $passed=$true
    Write-Output 'Compose initialization passed: Chat default, TLS, private secrets and no credential replacement.'
} finally {
    if($passed){
        if([IO.Path]::GetDirectoryName($fixture) -cne [IO.Path]::GetFullPath($ScratchDirectory) -or [IO.Path]::GetFileName($fixture) -cnotmatch '^compose-init-[a-f0-9]{32}$'){throw 'Fixture cleanup boundary changed'}
        foreach($item in Get-ChildItem -LiteralPath $fixture -Recurse -Force){if($item.Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'Linked fixture entry'}}
        Remove-Item -LiteralPath $fixture -Recurse -Force
    } else { Write-Output "Initialization test evidence retained at $fixture" }
}
