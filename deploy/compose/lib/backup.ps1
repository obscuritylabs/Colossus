function Get-ComposeContainer {
    param([string] $Docker, [string] $Root, $Manifest, [string] $Service)
    $id = (Invoke-Compose $Docker $Root $Manifest @('ps','--quiet',$Service)).Trim()
    if ($id -cnotmatch '^[a-f0-9]{12,64}$') { throw "The owned $Service service must be running for backup/restore." }
    return $id
}

function Invoke-ComposeDatabaseStream {
    param([string] $Docker, [string[]] $Arguments, [string] $OutputPath, [string] $InputPath)
    if ([bool]$OutputPath -eq [bool]$InputPath) { throw 'Select exactly one database stream direction.' }
    $start = [Diagnostics.ProcessStartInfo]::new($Docker)
    $start.UseShellExecute=$false; $start.CreateNoWindow=$true
    $start.RedirectStandardOutput=$true; $start.RedirectStandardError=$true; $start.RedirectStandardInput=[bool]$InputPath
    foreach($argument in $Arguments){[void]$start.ArgumentList.Add($argument)}
    $process=[Diagnostics.Process]::new();$process.StartInfo=$start
    $file=$null;$started=$false
    try {
        if($OutputPath){Assert-ComposePath $OutputPath;$file=[IO.File]::Open($OutputPath,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write)}
        else{Assert-ComposePath $InputPath;$file=[IO.File]::OpenRead($InputPath)}
        if(-not $process.Start()){throw 'Cannot start database stream.'}
        $started=$true
        $errorTask=$process.StandardError.ReadToEndAsync()
        if($OutputPath){$copy=$process.StandardOutput.BaseStream.CopyToAsync($file)}
        else{$outputTask=$process.StandardOutput.ReadToEndAsync();$copy=$file.CopyToAsync($process.StandardInput.BaseStream)}
        if(-not $copy.Wait(300000)){throw 'Database stream timed out.'}
        if($InputPath){$process.StandardInput.Close()}
        if(-not $process.WaitForExit(300000)){throw 'Database operation timed out.'}
        $errorText=$errorTask.GetAwaiter().GetResult()
        if($process.ExitCode -ne 0){
            if($script:ComposeFailureRoot){Write-ComposeFile (Join-Path $script:ComposeFailureRoot ('docker-failure-'+[Guid]::NewGuid().ToString('N')+'.log')) $errorText}
            throw 'Database backup/restore failed; original state, private staging, and diagnostics are retained.'
        }
        if($InputPath){$null=$outputTask.GetAwaiter().GetResult()}else{$file.Flush($true)}
    } finally {
        if($null -ne $file){$file.Dispose()}
        if($started -and -not $process.HasExited){$process.Kill($true);$process.WaitForExit()}
        $process.Dispose()
    }
}

function Backup-ComposeDeployment {
    param([string] $Docker, [string] $Root, $Manifest, [string] $Backup, [switch] $KeepStopped)
    Assert-ComposeManifest $Root $Manifest
    $null = Get-ComposeResources $Docker $Manifest
    $databases = @('postgres')
    if ($Manifest.profile -ceq 'hosted-demo') { $databases += 'anchor-postgres' }
    $containers = @{}
    foreach ($database in $databases) { $containers[$database] = Get-ComposeContainer $Docker $Root $Manifest $database }
    if (-not $Backup) {
        $Backup = Join-Path ([IO.Path]::GetDirectoryName($Root)) ('backups/' + $Manifest.name + '-' + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ') + '-' + [Guid]::NewGuid().ToString('N').Substring(0,8) + '.zip')
    }
    if (-not [IO.Path]::IsPathFullyQualified($Backup) -or [IO.Path]::GetExtension($Backup) -cne '.zip') { throw 'Backup must be an absolute new .zip filename.' }
    Assert-ComposePath $Backup
    if (Test-Path -LiteralPath $Backup) { throw 'Backup already exists; it will not be overwritten.' }
    $parent = [IO.Path]::GetDirectoryName($Backup)
    if (-not (Test-Path -LiteralPath $parent)) { [void][IO.Directory]::CreateDirectory($parent); Protect-ComposeDirectory $parent }
    Assert-ComposePrivateDirectory $parent
    $stage = Join-Path $parent ('.compose-backup-' + [Guid]::NewGuid().ToString('N'))
    [void][IO.Directory]::CreateDirectory($stage); Protect-ComposeDirectory $stage
    $stopped = @('control-plane')
    if ($Manifest.profile -ceq 'hosted-demo') { $stopped += 'anchor' }
    try {
        $null = Invoke-Compose $Docker $Root $Manifest (@('stop','--timeout','120') + $stopped)
        foreach ($database in $databases) {
            $id = $containers[$database]
            # Copy native byte streams directly. PowerShell 7.2 text pipelines and
            # Docker's archive API for tmpfs are not binary-safe backup transports.
            Invoke-ComposeDatabaseStream $Docker @('exec',$id,'pg_dump','-U','postgres','-d','colossus','--format=custom') -OutputPath (Join-Path $stage "$database.dump")
        }
        $optional = @('hosted-bootstrap.json','verification-intent.json','verification.json') | Where-Object { Test-Path -LiteralPath (Join-Path $Root $_) -PathType Leaf }
        foreach ($relative in (@($Manifest.files.Keys) + @('manifest.json') + @($optional))) {
            $destination = Join-Path $stage "deployment/$relative"
            [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination))
            [IO.File]::Copy((Join-Path $Root $relative), $destination, $false)
        }
        $files = @{}
        foreach ($item in Get-ChildItem -LiteralPath $stage -File -Recurse -Force) {
            $files[[IO.Path]::GetRelativePath($stage,$item.FullName).Replace('\','/')] = Get-ComposeHash $item.FullName
        }
        Write-ComposeJson (Join-Path $stage 'backup.json') @{ schemaVersion=1; purpose='ColossusComposeBackup'; deploymentId=$Manifest.deploymentId; createdAtUtc=[DateTime]::UtcNow.ToString('o'); files=$files }
        [IO.Compression.ZipFile]::CreateFromDirectory($stage,$Backup,[IO.Compression.CompressionLevel]::Optimal,$false)
        Write-ComposeFile ($Backup + '.sha256') ((Get-ComposeHash $Backup) + '  ' + [IO.Path]::GetFileName($Backup) + "`n")
        # Only this freshly allocated staging child is removed, after its archive
        # and checksum are durable. Failure evidence and the original remain.
        Assert-ComposePath $stage
        if ([IO.Path]::GetDirectoryName($stage) -cne $parent -or [IO.Path]::GetFileName($stage) -cnotmatch '^\.compose-backup-[a-f0-9]{32}$') { throw 'Backup staging boundary changed.' }
        foreach ($entry in Get-ChildItem -LiteralPath $stage -Recurse -Force) { if ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Linked backup staging entry.' } }
        Remove-Item -LiteralPath $stage -Recurse -Force
        Write-Output "Backup and checksum retained privately at $Backup. The archive contains credentials and keys."
    } finally {
        if (-not $KeepStopped) { $null = Invoke-Compose $Docker $Root $Manifest (@('start','--wait','--wait-timeout','240') + $stopped) }
    }
}

function Restore-ComposeBackup {
    param([string] $Docker, [string] $Root, [string] $Name, [string] $Backup, [ref] $OperationLock)
    if (-not $Backup -or -not [IO.Path]::IsPathFullyQualified($Backup)) { throw 'Restore requires an absolute -Backup path.' }
    Assert-ComposePath $Backup
    Assert-ComposePrivateDirectory ([IO.Path]::GetDirectoryName($Backup))
    $checksumPath = $Backup + '.sha256'
    Assert-ComposePath $checksumPath
    $checksum = [IO.File]::ReadAllText($checksumPath)
    if ($checksum -cnotmatch ('^([a-f0-9]{64})  ' + [regex]::Escape([IO.Path]::GetFileName($Backup)) + '\r?\n?$') -or (Get-ComposeHash $Backup) -cne $Matches[1]) { throw 'Backup checksum does not match.' }
    if (Test-Path -LiteralPath $Root) { throw 'Restore requires a fresh deployment name and absent directory.' }
    $parent = [IO.Path]::GetDirectoryName($Root)
    Assert-ComposePath $parent
    if (-not (Test-Path -LiteralPath $parent)) { [void][IO.Directory]::CreateDirectory($parent); Protect-ComposeDirectory $parent }
    Assert-ComposePrivateDirectory $parent
    # Atomically claim the new allocation; concurrent initialization cannot join it.
    $null = New-Item -ItemType Directory -Path $Root -ErrorAction Stop
    Protect-ComposeDirectory $Root
    if ($null -eq $OperationLock) { throw 'Restore requires its caller-held operation lock.' }
    $OperationLock.Value = [IO.File]::Open((Join-Path $Root '.operation.lock'),[IO.FileMode]::CreateNew,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
    $stage = Join-Path $Root 'restore-input'
    [void][IO.Directory]::CreateDirectory($stage); Protect-ComposeDirectory $stage
    $zip = [IO.Compression.ZipFile]::OpenRead($Backup)
    try {
        if ($zip.Entries.Count -gt 2048 -or (($zip.Entries | ForEach-Object Length | Measure-Object -Sum).Sum) -gt 16GB) { throw 'Backup exceeds restore limits.' }
        foreach ($entry in $zip.Entries) {
            $relative = $entry.FullName
            if ($relative.Contains('\') -or [IO.Path]::IsPathRooted($relative) -or $relative.Split('/') -contains '..' -or
                (($entry.ExternalAttributes -shr 16) -band 0xF000) -eq 0xA000 -or
                ($relative -cnotmatch '^(deployment/.+|postgres\.dump|anchor-postgres\.dump|backup\.json)$')) { throw 'Unsafe archive entry.' }
            if ($relative.EndsWith('/')) { continue }
            $destination = Join-Path $stage $relative
            if (-not [IO.Path]::GetFullPath($destination).StartsWith($stage + [IO.Path]::DirectorySeparatorChar,[StringComparison]::Ordinal)) { throw 'Archive path escapes restore allocation.' }
            [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination))
            $input = $entry.Open(); $output = [IO.File]::Open($destination,[IO.FileMode]::CreateNew)
            try { $input.CopyTo($output); $output.Flush($true) } finally { $input.Dispose(); $output.Dispose() }
        }
    } finally { $zip.Dispose() }
    $receipt = Get-Content -LiteralPath (Join-Path $stage 'backup.json') -Raw | ConvertFrom-Json -AsHashtable
    if ($receipt.schemaVersion -ne 1 -or $receipt.purpose -cne 'ColossusComposeBackup') { throw 'Unknown backup format.' }
    foreach ($item in Get-ChildItem -LiteralPath $stage -File -Recurse -Force | Where-Object FullName -CNE (Join-Path $stage 'backup.json')) {
        $relative = [IO.Path]::GetRelativePath($stage,$item.FullName).Replace('\','/')
        if (-not $receipt.files.ContainsKey($relative) -or (Get-ComposeHash $item.FullName) -cne $receipt.files[$relative]) { throw 'Backup contents do not match their receipt.' }
    }
    foreach ($relative in $receipt.files.Keys) { if (-not (Test-Path -LiteralPath (Join-Path $stage $relative) -PathType Leaf)) { throw 'Backup is incomplete.' } }
    $oldRoot = Join-Path $stage 'deployment'
    $manifest = Get-Content -LiteralPath (Join-Path $oldRoot 'manifest.json') -Raw | ConvertFrom-Json -AsHashtable
    if ($manifest.schemaVersion -ne 1 -or $manifest.purpose -cne 'ColossusCompose' -or $manifest.deploymentId -cne $receipt.deploymentId) { throw 'Backup deployment identity mismatch.' }
    foreach ($relative in $manifest.files.Keys) {
        if ([IO.Path]::IsPathRooted($relative) -or $relative.Split([char[]]@('/','\')) -contains '..') { throw 'Invalid backed-up material path.' }
        $source = Join-Path $oldRoot $relative
        if ((Get-ComposeHash $source) -cne $manifest.files[$relative]) { throw 'Backed-up deployment material is incomplete.' }
        $destination = Join-Path $Root $relative
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($destination))
        [IO.File]::Copy($source,$destination,$false)
    }
    foreach ($relative in @('hosted-bootstrap.json','verification-intent.json','verification.json')) {
        $source = Join-Path $oldRoot $relative
        if (Test-Path -LiteralPath $source) { [IO.File]::Copy($source,(Join-Path $Root $relative),$false) }
    }
    $manifest.deploymentId = [Guid]::NewGuid().ToString('N'); $manifest.owner = Get-ComposeOwner
    $manifest.name = $Name; $manifest.project = "colossus-$Name-$($manifest.deploymentId.Substring(0,8))"
    $manifest.root = $Root; $manifest.engineId = ''; $manifest.provisioned = $false; $manifest.dataRemoved = $false
    $manifest['restoring'] = $true
    $manifest['restoredFrom'] = @{ deploymentId=$receipt.deploymentId; sha256=(Get-ComposeHash $Backup) }
    Write-ComposeJson (Join-Path $Root 'manifest.json') $manifest
    Save-ComposeSelection $Root $manifest
    Assert-ComposeManifest $Root $manifest
    Assert-ComposeEngine $Docker $Root $manifest
    Start-ComposeDeployment $Docker $Root $manifest -DatabaseOnly
    $databases = @('postgres')
    if ($manifest.profile -ceq 'hosted-demo') { $databases += 'anchor-postgres' }
    foreach ($database in $databases) {
        $id = Get-ComposeContainer $Docker $Root $manifest $database
        Invoke-ComposeDatabaseStream $Docker @('exec','--interactive',$id,'pg_restore','-U','postgres','-d','colossus','--clean','--if-exists','--no-owner','--role=colossus','--exit-on-error') -InputPath (Join-Path $stage "$database.dump")
    }
    $manifest.restoring = $false
    Write-ComposeJson (Join-Path $Root 'manifest.json') $manifest
    # Keep the protected restore input as recovery evidence until the operator
    # verifies the new installation. It is never used by running containers.
    return $manifest
}
