function New-ColossusReleaseFixture {
    if (-not $IsWindows) { throw "release fixtures require Windows" }
    $fixtureProfilePath = [Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)
    if ([string]::IsNullOrWhiteSpace($fixtureProfilePath) -or -not [IO.Path]::IsPathFullyQualified($fixtureProfilePath)) {
        throw "the current Windows user profile is unavailable"
    }
    $root = Join-Path $fixtureProfilePath ("colossus-release-fixture-" + [Guid]::NewGuid().ToString("N"))
    # Match the native acceptance fixtures: RUNNER_TEMP can inherit a shared DACL.
    # Never change its ACL or repair any existing user directory.
    New-Item -ItemType Directory -Path $root -ErrorAction Stop | Out-Null
    try {
        $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
        try { $owner = $identity.User } finally { $identity.Dispose() }
        $acl = [Security.AccessControl.DirectorySecurity]::new()
        $acl.SetOwner($owner)
        $acl.SetAccessRuleProtection($true, $false)
        $inheritance = [Security.AccessControl.InheritanceFlags]::ContainerInherit -bor
            [Security.AccessControl.InheritanceFlags]::ObjectInherit
        foreach ($sid in @($owner, [Security.Principal.SecurityIdentifier]::new("S-1-5-18"),
                [Security.Principal.SecurityIdentifier]::new("S-1-5-32-544"))) {
            $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
                $sid, [Security.AccessControl.FileSystemRights]::FullControl, $inheritance,
                [Security.AccessControl.PropagationFlags]::None,
                [Security.AccessControl.AccessControlType]::Allow
            )) | Out-Null
        }
        Set-Acl -LiteralPath $root -AclObject $acl -ErrorAction Stop
        return $root
    } catch {
        Remove-Item -LiteralPath $root -Force -ErrorAction SilentlyContinue
        throw
    }
}
