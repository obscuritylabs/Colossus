Set-StrictMode -Version Latest

function Get-ComposeOwner {
    if ($IsWindows) { return [Security.Principal.WindowsIdentity]::GetCurrent().User.Value }
    return (& id -u).Trim()
}

function Assert-ComposePath {
    param([string] $Path)
    if (-not [IO.Path]::IsPathFullyQualified($Path)) { throw 'Deployment paths must be absolute.' }
    $cursor = [IO.Path]::GetFullPath($Path)
    while ($cursor) {
        if (Test-Path -LiteralPath $cursor) {
            $item = Get-Item -LiteralPath $cursor -Force
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Linked deployment paths are not supported.' }
        }
        $cursor = [IO.Path]::GetDirectoryName($cursor)
    }
}

function Protect-ComposeDirectory {
    param([string] $Path)
    Assert-ComposePath $Path
    if ($IsWindows) {
        $acl = [Security.AccessControl.DirectorySecurity]::new()
        $acl.SetAccessRuleProtection($true, $false)
        $user = [Security.Principal.WindowsIdentity]::GetCurrent().User
        $acl.SetOwner($user)
        foreach ($sid in @($user, [Security.Principal.SecurityIdentifier]::new('S-1-5-18'))) {
            $rule = [Security.AccessControl.FileSystemAccessRule]::new($sid, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow')
            [void]$acl.AddAccessRule($rule)
        }
        Set-Acl -LiteralPath $Path -AclObject $acl
    } else {
        & chmod 700 -- $Path
        if ($LASTEXITCODE -ne 0) { throw 'Cannot protect deployment directory.' }
    }
    Assert-ComposePrivateDirectory $Path
}

function Assert-ComposePrivateDirectory {
    param([string] $Path)
    Assert-ComposePath $Path
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) { throw 'Retained deployment directory is missing.' }
    if ($IsWindows) {
        $acl = Get-Acl -LiteralPath $Path
        $owner = ([Security.Principal.NTAccount]$acl.Owner).Translate([Security.Principal.SecurityIdentifier]).Value
        $allowed = @((Get-ComposeOwner), 'S-1-5-18')
        if ($owner -cne $allowed[0] -or -not $acl.AreAccessRulesProtected) { throw 'Deployment directory must have its current-owner private ACL.' }
        foreach ($rule in $acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])) {
            if ($rule.AccessControlType -eq 'Allow' -and $rule.IdentityReference.Value -notin $allowed) { throw 'Deployment directory grants another identity access.' }
        }
    } else {
        $item = Get-Item -LiteralPath $Path
        if ($item.User -cne (Get-ComposeOwner) -and (& id -u $item.User).Trim() -cne (Get-ComposeOwner)) { throw 'Deployment directory belongs to another user.' }
        # PowerShell 7.2 runs on .NET 6; avoid newer GetUnixFileMode APIs.
        $mode = if ($IsMacOS) { (& stat -f '%Lp' -- $Path).Trim() } else { (& stat -c '%a' -- $Path).Trim() }
        if ($LASTEXITCODE -ne 0 -or $mode -cne '700') { throw 'Deployment directory permissions must be 0700.' }
    }
}

function Write-ComposeFile {
    param([string] $Path, [string] $Text)
    Assert-ComposePath $Path
    $stream = [IO.File]::Open($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        $bytes = [Text.UTF8Encoding]::new($false).GetBytes($Text.Replace("`r`n", "`n"))
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
    } finally { $stream.Dispose() }
    if (-not $IsWindows) { & chmod 600 -- $Path; if ($LASTEXITCODE -ne 0) { throw 'Cannot protect deployment file.' } }
}

function Write-ComposeJson {
    param([string] $Path, $Value)
    $temporary = $Path + '.' + [Guid]::NewGuid().ToString('N') + '.tmp'
    Write-ComposeFile $temporary (($Value | ConvertTo-Json -Depth 40) + "`n")
    [IO.File]::Move($temporary, $Path, $true)
}

function Get-ComposeHash {
    param([string] $Path)
    Assert-ComposePath $Path
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Get-ComposeRoot {
    param([string] $Name, [string] $StateDirectory)
    if ($Name -cnotmatch '^[a-z][a-z0-9-]{0,23}$') { throw 'Invalid deployment name.' }
    $parent = if ($StateDirectory) { [IO.Path]::GetFullPath($StateDirectory) } else {
        Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)) '.colossus-deployments'
    }
    Assert-ComposePath $parent
    return Join-Path $parent "compose-$Name"
}

function Assert-ComposeManifest {
    param([string] $Root, $Manifest)
    Assert-ComposePrivateDirectory $Root
    if ($Manifest.schemaVersion -ne 1 -or $Manifest.purpose -cne 'ColossusCompose' -or
        $Manifest.owner -cne (Get-ComposeOwner) -or $Manifest.deploymentId -cnotmatch '^[a-f0-9]{32}$' -or
        $Manifest.name -cnotmatch '^[a-z][a-z0-9-]{0,23}$' -or
        $Manifest.project -cne "colossus-$($Manifest.name)-$($Manifest.deploymentId.Substring(0,8))" -or
        [IO.Path]::GetFullPath([string]$Manifest.root) -cne [IO.Path]::GetFullPath($Root) -or
        $Manifest.profile -cnotin @('core','hosted-demo')) { throw 'Deployment ownership receipt does not match this allocation.' }
    foreach ($entry in $Manifest.files.GetEnumerator()) {
        $relative = [string]$entry.Key
        if ([IO.Path]::IsPathRooted($relative) -or $relative.Split([char[]]@('/','\')) -contains '..') { throw 'Invalid retained deployment filename.' }
        $path = Join-Path $Root $relative
        if (-not (Test-Path -LiteralPath $path -PathType Leaf) -or (Get-ComposeHash $path) -cne $entry.Value) { throw 'Retained deployment material changed or is missing; credentials will not be regenerated.' }
    }
}

function Assert-ComposeCertificates {
    param([string] $Root)
    # Expired leaves must not prevent an operator from backing up or stopping
    # an installation. Only startup requires usable service certificates.
    foreach ($role in @('browser','database','controller','anchor','anchor-database','anchor-browser')) {
        $certificate = Join-Path $Root "material/$role/server.pem"
        if (Test-Path -LiteralPath $certificate) {
            $cert = [Security.Cryptography.X509Certificates.X509Certificate2]::new($certificate)
            try { if ($cert.NotAfter.ToUniversalTime() -lt [DateTime]::UtcNow.AddDays(1)) { throw 'Retained TLS certificate requires deliberate renewal.' } } finally { $cert.Dispose() }
        }
    }
}
