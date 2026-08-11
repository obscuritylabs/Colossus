param(
    [string]$Prefix = (Join-Path $HOME ".local")
)

$ErrorActionPreference = "Stop"

function Throw-InstallerError([string]$Message) {
    throw "colossus package installer: $Message"
}

if ([string]::IsNullOrWhiteSpace($Prefix) -or -not [IO.Path]::IsPathRooted($Prefix)) {
    Throw-InstallerError "install prefix must be an absolute path"
}
if ($Prefix.IndexOfAny([char[]](0..31)) -ge 0) {
    Throw-InstallerError "install prefix cannot contain control characters"
}

function Assert-NoReparseComponents([string]$Path) {
    $current = [IO.Path]::GetFullPath($Path)
    while (-not [string]::IsNullOrEmpty($current)) {
        if (Test-Path -LiteralPath $current) {
            $item = Get-Item -LiteralPath $current -Force
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                Throw-InstallerError "refusing to install through a link or reparse point: $current"
            }
        }
        $parent = [IO.Directory]::GetParent($current)
        if ($null -eq $parent) { break }
        $current = $parent.FullName
    }
}

function Assert-OwnedDirectory([string]$Path) {
    $item = Get-Item -LiteralPath $Path -Force
    if (-not $item.PSIsContainer -or
        ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        Throw-InstallerError "installation directory is missing, linked, or not a directory: $Path"
    }
    $currentIdentity = [Security.Principal.WindowsIdentity]::GetCurrent()
    try {
        $currentOwnerSid = $currentIdentity.Owner
    } finally {
        $currentIdentity.Dispose()
    }
    if ($null -eq $currentOwnerSid) {
        Throw-InstallerError "current Windows token has no owner SID"
    }
    $ownerSid = (Get-Acl -LiteralPath $Path).GetOwner(
        [Security.Principal.SecurityIdentifier]
    )
    if (-not $ownerSid.Equals($currentOwnerSid)) {
        Throw-InstallerError "installation directory is not owned by the current user: $Path"
    }
}

$sourceBinary = Join-Path $PSScriptRoot "colossus.exe"
$metadataPath = Join-Path $PSScriptRoot "install-metadata"
if (-not (Test-Path -LiteralPath $sourceBinary -PathType Leaf)) {
    Throw-InstallerError "package colossus.exe is missing"
}
$sourceItem = Get-Item -LiteralPath $sourceBinary -Force
if (($sourceItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
    Throw-InstallerError "package colossus.exe cannot be a link or reparse point"
}
if (-not (Test-Path -LiteralPath $metadataPath -PathType Leaf)) {
    Throw-InstallerError "package installation metadata is missing"
}
$metadataItem = Get-Item -LiteralPath $metadataPath -Force
if (($metadataItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
    Throw-InstallerError "package installation metadata cannot be a link or reparse point"
}

$metadata = @{}
foreach ($line in Get-Content -LiteralPath $metadataPath) {
    $parts = $line.Split('=', 2)
    if ($parts.Count -ne 2 -or $metadata.ContainsKey($parts[0])) {
        Throw-InstallerError "package metadata has an invalid shape"
    }
    $metadata[$parts[0]] = $parts[1]
}
$metadataFields = @(
    "schema_version",
    "version",
    "target",
    "channel",
    "distribution_origin",
    "installer_kind"
)
if ($metadata.Count -ne $metadataFields.Count) {
    Throw-InstallerError "package metadata must contain exactly six fields"
}
foreach ($field in $metadataFields) {
    if (-not $metadata.ContainsKey($field)) {
        Throw-InstallerError "package metadata is missing $field"
    }
}
if ($metadata.schema_version -cne "1") {
    Throw-InstallerError "package metadata schema is unsupported"
}
$stablePattern = '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'
$previewPattern = '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)-preview\.([1-9][0-9]*)$'
if (($metadata.channel -ceq "stable" -and $metadata.version -notmatch $stablePattern) -or
    ($metadata.channel -ceq "preview" -and $metadata.version -notmatch $previewPattern)) {
    Throw-InstallerError "package metadata channel and version disagree"
}
if ($metadata.target -cnotin @("aarch64-pc-windows-msvc", "x86_64-pc-windows-msvc")) {
    Throw-InstallerError "package metadata target is invalid"
}
if ($metadata.distribution_origin -cne "https://github.com/obscuritylabs/Colossus/releases") {
    Throw-InstallerError "package metadata distribution origin is invalid"
}
if ($metadata.installer_kind -cne "direct") {
    Throw-InstallerError "package metadata installer kind is invalid"
}

$binaryVersion = (& $sourceBinary --version | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or $binaryVersion -cne "colossus $($metadata.version)") {
    Throw-InstallerError "package binary version disagrees with metadata"
}

$binDirectory = Join-Path $Prefix "bin"
Assert-NoReparseComponents $binDirectory
New-Item -ItemType Directory -Path $binDirectory -Force | Out-Null
Assert-NoReparseComponents $binDirectory
Assert-OwnedDirectory $binDirectory

$receiptRoot = $env:LOCALAPPDATA
if ([string]::IsNullOrWhiteSpace($receiptRoot)) {
    if ([string]::IsNullOrWhiteSpace($HOME)) {
        Throw-InstallerError "LOCALAPPDATA or HOME must be set for the installation receipt"
    }
    $receiptRoot = Join-Path $HOME "AppData/Local"
}
if (-not [IO.Path]::IsPathRooted($receiptRoot)) {
    Throw-InstallerError "installation receipt root must be absolute"
}
$receiptDirectory = Join-Path $receiptRoot "Colossus"
Assert-NoReparseComponents $receiptDirectory
New-Item -ItemType Directory -Path $receiptDirectory -Force | Out-Null
Assert-NoReparseComponents $receiptDirectory
Assert-OwnedDirectory $receiptDirectory

$target = Join-Path $binDirectory "colossus.exe"
$receipt = Join-Path $receiptDirectory "install.json"
foreach ($existingPath in @($target, $receipt)) {
    if (Test-Path -LiteralPath $existingPath) {
        $existing = Get-Item -LiteralPath $existingPath -Force
        if ($existing.PSIsContainer -or
            ($existing.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            Throw-InstallerError "existing installation path is linked, reparsed, or non-regular: $existingPath"
        }
    }
}

$temporary = Join-Path $binDirectory (".colossus.install." + [Guid]::NewGuid() + ".exe")
$backup = Join-Path $binDirectory (".colossus.backup." + [Guid]::NewGuid() + ".exe")
$temporaryReceipt = Join-Path $receiptDirectory (".install.json." + [Guid]::NewGuid())
$receiptBackup = Join-Path $receiptDirectory (".install.backup." + [Guid]::NewGuid())
$hadExistingBinary = Test-Path -LiteralPath $target
$hadExistingReceipt = Test-Path -LiteralPath $receipt
$binaryCommitted = $false
$receiptCommitted = $false
try {
    Copy-Item -LiteralPath $sourceBinary -Destination $temporary
    $receiptDocument = [ordered]@{
        schemaVersion = 1
        channel = $metadata.channel
        version = $metadata.version
        target = $metadata.target
        prefix = [IO.Path]::GetFullPath($Prefix)
        binaryPath = [IO.Path]::GetFullPath($target)
        distributionOrigin = $metadata.distribution_origin
        installerKind = $metadata.installer_kind
    }
    $receiptJson = $receiptDocument | ConvertTo-Json
    [IO.File]::WriteAllText(
        $temporaryReceipt,
        "$receiptJson`n",
        [Text.UTF8Encoding]::new($false)
    )

    if ($hadExistingBinary) {
        [IO.File]::Replace($temporary, $target, $backup)
    } else {
        [IO.File]::Move($temporary, $target)
    }
    $binaryCommitted = $true

    try {
        if ($hadExistingReceipt) {
            [IO.File]::Replace($temporaryReceipt, $receipt, $receiptBackup)
        } else {
            [IO.File]::Move($temporaryReceipt, $receipt)
        }
        $receiptCommitted = $true
    } catch {
        if ($hadExistingBinary -and (Test-Path -LiteralPath $backup)) {
            [IO.File]::Replace($backup, $target, $null)
        } elseif (Test-Path -LiteralPath $target) {
            Remove-Item -LiteralPath $target -Force
        }
        $binaryCommitted = $false
        Throw-InstallerError "installation receipt could not be committed; the binary was rolled back"
    }
} finally {
    if ($binaryCommitted -and -not $receiptCommitted) {
        if ($hadExistingBinary -and (Test-Path -LiteralPath $backup)) {
            [IO.File]::Replace($backup, $target, $null)
        } elseif (Test-Path -LiteralPath $target) {
            Remove-Item -LiteralPath $target -Force
        }
        $binaryCommitted = $false
    }
    foreach ($temporaryPath in @($temporary, $backup, $temporaryReceipt, $receiptBackup)) {
        Remove-Item -LiteralPath $temporaryPath -Force -ErrorAction SilentlyContinue
    }
}

if (-not $binaryCommitted) {
    Throw-InstallerError "installation did not commit"
}
Write-Output "installed $target"
Write-Output "recorded direct installation receipt at $receipt"
