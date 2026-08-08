[CmdletBinding()]
param(
    [string]$Version,
    [string]$Prefix,
    [ValidateSet("stable", "preview")]
    [string]$Channel = "stable",
    [switch]$DryRun,
    [switch]$NoModifyPath,
    [switch]$Yes
)

$ErrorActionPreference = "Stop"
$repository = "obscuritylabs/Colossus"
$apiOrigin = "https://api.github.com"
$releaseOrigin = "https://github.com/obscuritylabs/Colossus/releases"
$maximumMetadataBytes = 1MB
$maximumChecksumBytes = 512
$maximumArchiveBytes = 256MB

function Throw-InstallerError([string]$Message) {
    throw "colossus installer: $Message"
}

if ([string]::IsNullOrWhiteSpace($Prefix)) {
    if ([string]::IsNullOrWhiteSpace($HOME)) {
        Throw-InstallerError "HOME must be set when -Prefix is omitted"
    }
    $Prefix = Join-Path $HOME ".local"
}
if (-not [IO.Path]::IsPathRooted($Prefix)) {
    Throw-InstallerError "install prefix must be absolute"
}

$stableVersionPattern = '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'
$previewVersionPattern = '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)-preview\.([1-9][0-9]*)$'
if (-not [string]::IsNullOrEmpty($Version)) {
    if (($Channel -eq "stable" -and $Version -notmatch $stableVersionPattern) -or
        ($Channel -eq "preview" -and $Version -notmatch $previewVersionPattern)) {
        Throw-InstallerError "requested version does not match the selected channel"
    }
}

if (-not [Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
        [Runtime.InteropServices.OSPlatform]::Windows
    )) {
    Throw-InstallerError "the PowerShell installer supports Windows only"
}
$architecture = [Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
switch ($architecture) {
    "Arm64" { $target = "aarch64-pc-windows-msvc" }
    "X64" { $target = "x86_64-pc-windows-msvc" }
    default { Throw-InstallerError "unsupported Windows architecture: $architecture" }
}

$temporaryRoot = Join-Path ([IO.Path]::GetTempPath()) ("colossus-install." + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $temporaryRoot | Out-Null

Add-Type -AssemblyName System.Net.Http
$handler = [Net.Http.HttpClientHandler]::new()
$handler.AllowAutoRedirect = $false
$handler.UseProxy = $false
$client = [Net.Http.HttpClient]::new($handler)
$client.Timeout = [TimeSpan]::FromMinutes(5)
$client.DefaultRequestHeaders.UserAgent.ParseAdd("colossus-bootstrap-installer/1")

function Invoke-BoundedDownload {
    param(
        [Parameter(Mandatory = $true)][Uri]$Uri,
        [Parameter(Mandatory = $true)][string]$Destination,
        [Parameter(Mandatory = $true)][long]$MaximumBytes,
        [Parameter(Mandatory = $true)][string[]]$AllowedHosts,
        [Parameter(Mandatory = $true)][int]$MaximumRedirects,
        [switch]$GitHubMetadata
    )

    $current = $Uri
    for ($redirects = 0; ; $redirects++) {
        $request = [Net.Http.HttpRequestMessage]::new([Net.Http.HttpMethod]::Get, $current)
        if ($GitHubMetadata) {
            $request.Headers.Accept.ParseAdd("application/vnd.github+json")
            $request.Headers.Add("X-GitHub-Api-Version", "2022-11-28")
        }
        try {
            $response = $client.SendAsync(
                $request,
                [Net.Http.HttpCompletionOption]::ResponseHeadersRead
            ).GetAwaiter().GetResult()
        } finally {
            $request.Dispose()
        }
        try {
            $status = [int]$response.StatusCode
            if ($status -in 301, 302, 303, 307, 308) {
                if ($redirects -ge $MaximumRedirects -or $null -eq $response.Headers.Location) {
                    Throw-InstallerError "download redirected unexpectedly"
                }
                $next = [Uri]::new($current, $response.Headers.Location)
                if ($next.Scheme -ne "https" -or -not $next.IsDefaultPort -or
                    $AllowedHosts -notcontains $next.DnsSafeHost) {
                    Throw-InstallerError "download redirected to an unexpected origin"
                }
                $current = $next
                continue
            }
            if ($status -ne 200) {
                Throw-InstallerError "download failed with HTTP status $status"
            }
            $declaredLength = $response.Content.Headers.ContentLength
            if ($null -ne $declaredLength -and $declaredLength -gt $MaximumBytes) {
                Throw-InstallerError "download is larger than its fixed limit"
            }
            $source = $response.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
            $destinationStream = [IO.File]::Open(
                $Destination,
                [IO.FileMode]::Create,
                [IO.FileAccess]::Write,
                [IO.FileShare]::None
            )
            try {
                $buffer = [byte[]]::new(65536)
                [long]$total = 0
                while (($read = $source.Read($buffer, 0, $buffer.Length)) -gt 0) {
                    $total += $read
                    if ($total -gt $MaximumBytes) {
                        Throw-InstallerError "download is larger than its fixed limit"
                    }
                    $destinationStream.Write($buffer, 0, $read)
                }
            } finally {
                $destinationStream.Dispose()
                $source.Dispose()
            }
            return $current
        } finally {
            $response.Dispose()
        }
    }
}

function Get-BoundedJson([string]$Url, [string]$Name) {
    $path = Join-Path $temporaryRoot $Name
    $uri = [Uri]$Url
    if ($uri.Scheme -ne "https" -or $uri.DnsSafeHost -ne "api.github.com" -or
        -not $uri.IsDefaultPort) {
        Throw-InstallerError "release metadata origin is invalid"
    }
    $effective = Invoke-BoundedDownload `
        -Uri $uri `
        -Destination $path `
        -MaximumBytes $maximumMetadataBytes `
        -AllowedHosts @("api.github.com") `
        -MaximumRedirects 0 `
        -GitHubMetadata
    if ($effective.AbsoluteUri -ne $uri.AbsoluteUri) {
        Throw-InstallerError "release metadata redirected unexpectedly"
    }
    try {
        return Get-Content -LiteralPath $path -Raw -Encoding UTF8 | ConvertFrom-Json
    } catch {
        Throw-InstallerError "release metadata is not valid bounded JSON"
    }
}

function Assert-ReleaseIdentity($Release, [string]$ExpectedTag, [bool]$Prerelease) {
    if ($Release.tag_name -cne $ExpectedTag) {
        Throw-InstallerError "release metadata tag disagrees with the requested version"
    }
    if ($Release.draft -ne $false) {
        Throw-InstallerError "draft releases cannot be installed"
    }
    if ($Release.prerelease -ne $Prerelease) {
        Throw-InstallerError "release metadata disagrees with the requested channel"
    }
}

try {
    if (-not [string]::IsNullOrEmpty($Version)) {
        $release = Get-BoundedJson "$apiOrigin/repos/$repository/releases/tags/$Version" "release.json"
        Assert-ReleaseIdentity $release $Version ($Channel -eq "preview")
        $releaseTag = $Version
    } elseif ($Channel -eq "stable") {
        $release = Get-BoundedJson "$apiOrigin/repos/$repository/releases/latest" "release.json"
        $releaseTag = [string]$release.tag_name
        if ($releaseTag -notmatch $stableVersionPattern) {
            Throw-InstallerError "latest stable release returned an invalid tag"
        }
        Assert-ReleaseIdentity $release $releaseTag $false
    } else {
        $releases = @(
            Get-BoundedJson "$apiOrigin/repos/$repository/releases?per_page=20" "releases.json"
        )
        $release = $releases |
            Where-Object {
                $_.tag_name -match $previewVersionPattern -and
                $_.draft -eq $false -and
                $_.prerelease -eq $true
            } |
            Select-Object -First 1
        if ($null -eq $release) {
            Throw-InstallerError "no published preview release was found in the bounded release window"
        }
        $releaseTag = [string]$release.tag_name
        $release = Get-BoundedJson "$apiOrigin/repos/$repository/releases/tags/$releaseTag" "release.json"
        Assert-ReleaseIdentity $release $releaseTag $true
    }

    $resolvedVersion = $releaseTag.Substring(1)
    $archive = "colossus-$resolvedVersion-$target.zip"
    $checksum = "$archive.sha256"
    $assetNames = @($release.assets | ForEach-Object { [string]$_.name })
    if ($assetNames -cnotcontains $archive) {
        Throw-InstallerError "release metadata omits $archive"
    }
    if ($assetNames -cnotcontains $checksum) {
        Throw-InstallerError "release metadata omits $checksum"
    }

    Write-Output "Colossus install plan"
    Write-Output "  channel: $Channel"
    Write-Output "  version: $releaseTag"
    Write-Output "  target: $target"
    Write-Output "  prefix: $Prefix"
    Write-Output "  archive: $archive"
    if ($DryRun) {
        Write-Output "dry run complete; no archive was downloaded and no files were changed"
        return
    }

    $archivePath = Join-Path $temporaryRoot $archive
    $checksumPath = Join-Path $temporaryRoot $checksum
    $assetBase = "$releaseOrigin/download/$releaseTag"
    Invoke-BoundedDownload `
        -Uri ([Uri]"$assetBase/$archive") `
        -Destination $archivePath `
        -MaximumBytes $maximumArchiveBytes `
        -AllowedHosts @("github.com", "release-assets.githubusercontent.com") `
        -MaximumRedirects 3 | Out-Null
    Invoke-BoundedDownload `
        -Uri ([Uri]"$assetBase/$checksum") `
        -Destination $checksumPath `
        -MaximumBytes $maximumChecksumBytes `
        -AllowedHosts @("github.com", "release-assets.githubusercontent.com") `
        -MaximumRedirects 3 | Out-Null

    $checksumText = Get-Content -LiteralPath $checksumPath -Raw -Encoding ASCII
    $match = [regex]::Match(
        $checksumText,
        '\A([0-9a-f]{64})  ([A-Za-z0-9._-]+)\r?\n?\z',
        [Text.RegularExpressions.RegexOptions]::CultureInvariant
    )
    if (-not $match.Success -or $match.Groups[2].Value -cne $archive) {
        Throw-InstallerError "checksum sidecar has an invalid shape"
    }
    $expectedDigest = $match.Groups[1].Value
    $actualDigest = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualDigest -cne $expectedDigest) {
        Throw-InstallerError "archive checksum mismatch"
    }

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $package = "colossus-$resolvedVersion-$target"
    $expectedEntries = @(
        "$package/colossus.exe",
        "$package/install-metadata",
        "$package/install.ps1",
        "$package/LICENSE",
        "$package/README.md"
    )
    $seen = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
    $zip = [IO.Compression.ZipFile]::OpenRead($archivePath)
    try {
        [long]$expandedBytes = 0
        foreach ($entry in $zip.Entries) {
            $name = $entry.FullName.Replace('\', '/')
            if (-not $seen.Add($name)) {
                Throw-InstallerError "archive contains duplicate paths"
            }
            if ($name.StartsWith('/') -or $name.Contains('../') -or $name.Contains(':')) {
                Throw-InstallerError "archive contains an unsafe path"
            }
            $unixType = (($entry.ExternalAttributes -shr 16) -band 0xF000)
            $windowsAttributes = $entry.ExternalAttributes -band 0xFFFF
            if ($unixType -eq 0xA000 -or
                ($windowsAttributes -band [int][IO.FileAttributes]::ReparsePoint) -ne 0) {
                Throw-InstallerError "archive contains a link or reparse point"
            }
            if ($name -eq "$package/") {
                continue
            }
            if ($expectedEntries -cnotcontains $name -or $entry.Name.Length -eq 0) {
                Throw-InstallerError "archive layout contains missing or unexpected paths"
            }
            $expandedBytes += $entry.Length
            if ($expandedBytes -gt $maximumArchiveBytes) {
                Throw-InstallerError "expanded archive is larger than its fixed limit"
            }
        }
        foreach ($expected in $expectedEntries) {
            if (-not $seen.Contains($expected)) {
                Throw-InstallerError "archive layout is missing $expected"
            }
        }

        $extractRoot = Join-Path $temporaryRoot "extract"
        $packageRoot = Join-Path $extractRoot $package
        New-Item -ItemType Directory -Path $packageRoot | Out-Null
        foreach ($entry in $zip.Entries) {
            $name = $entry.FullName.Replace('\', '/')
            if ($name -eq "$package/") { continue }
            $leaf = $name.Substring($package.Length + 1)
            $destination = Join-Path $packageRoot $leaf
            $source = $entry.Open()
            $output = [IO.File]::Open(
                $destination,
                [IO.FileMode]::CreateNew,
                [IO.FileAccess]::Write,
                [IO.FileShare]::None
            )
            try {
                $source.CopyTo($output)
            } finally {
                $output.Dispose()
                $source.Dispose()
            }
        }
    } finally {
        $zip.Dispose()
    }

    foreach ($leaf in @("colossus.exe", "install-metadata", "install.ps1", "LICENSE", "README.md")) {
        $item = Get-Item -LiteralPath (Join-Path $packageRoot $leaf) -Force
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            Throw-InstallerError "extracted package contains a reparse point"
        }
    }

    $metadata = @{}
    foreach ($line in Get-Content -LiteralPath (Join-Path $packageRoot "install-metadata")) {
        $parts = $line.Split('=', 2)
        if ($parts.Count -ne 2 -or $metadata.ContainsKey($parts[0])) {
            Throw-InstallerError "package metadata has an invalid shape"
        }
        $metadata[$parts[0]] = $parts[1]
    }
    $expectedMetadata = [ordered]@{
        schema_version = "1"
        version = $resolvedVersion
        target = $target
        channel = $Channel
        distribution_origin = $releaseOrigin
        installer_kind = "direct"
    }
    if ($metadata.Count -ne $expectedMetadata.Count) {
        Throw-InstallerError "package metadata has an invalid field count"
    }
    foreach ($field in $expectedMetadata.Keys) {
        if ($metadata[$field] -cne $expectedMetadata[$field]) {
            Throw-InstallerError "package metadata mismatch for $field"
        }
    }

    $binary = Join-Path $packageRoot "colossus.exe"
    $binaryVersion = (& $binary --version | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $binaryVersion -cne "colossus $resolvedVersion") {
        Throw-InstallerError "downloaded binary version mismatch"
    }

    & (Join-Path $packageRoot "install.ps1") -Prefix $Prefix
    if ($LASTEXITCODE -ne 0) {
        Throw-InstallerError "packaged installer failed"
    }

    $binDirectory = Join-Path $Prefix "bin"
    $pathEntries = @($env:PATH -split ';')
    if ($pathEntries -cnotcontains $binDirectory) {
        Write-Output "Add Colossus to this shell with:"
        Write-Output ('  $env:PATH = "' + $binDirectory + ';$env:PATH"')
    }

    # The flags remain stable for unattended invocations. This installer never writes
    # a PowerShell profile, so neither flag grants implicit profile-write authority.
    $null = $NoModifyPath
    $null = $Yes
} finally {
    $client.Dispose()
    $handler.Dispose()
    if (Test-Path -LiteralPath $temporaryRoot) {
        Remove-Item -LiteralPath $temporaryRoot -Recurse -Force
    }
}
