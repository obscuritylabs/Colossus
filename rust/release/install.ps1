param(
    [string]$Prefix = (Join-Path $HOME ".local")
)

$ErrorActionPreference = "Stop"
if ([string]::IsNullOrWhiteSpace($Prefix)) {
    throw "install prefix cannot be empty"
}

$sourceBinary = Join-Path $PSScriptRoot "colossus.exe"
if (-not (Test-Path -LiteralPath $sourceBinary -PathType Leaf)) {
    throw "package colossus.exe is missing"
}
$sourceItem = Get-Item -LiteralPath $sourceBinary -Force
if (($sourceItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw "package colossus.exe cannot be a link or reparse point"
}

$binDirectory = Join-Path $Prefix "bin"
if (Test-Path -LiteralPath $binDirectory) {
    $binItem = Get-Item -LiteralPath $binDirectory -Force
    if (($binItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "refusing to install through a linked bin directory: $binDirectory"
    }
} else {
    New-Item -ItemType Directory -Path $binDirectory -Force | Out-Null
}

$target = Join-Path $binDirectory "colossus.exe"
$temporary = Join-Path $binDirectory (".colossus.install." + [Guid]::NewGuid() + ".exe")
$backup = Join-Path $binDirectory (".colossus.backup." + [Guid]::NewGuid() + ".exe")
try {
    Copy-Item -LiteralPath $sourceBinary -Destination $temporary
    if (Test-Path -LiteralPath $target) {
        [IO.File]::Replace($temporary, $target, $backup)
        Remove-Item -LiteralPath $backup -Force
    } else {
        Move-Item -LiteralPath $temporary -Destination $target
    }
} finally {
    Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $backup -Force -ErrorAction SilentlyContinue
}

Write-Output "installed $target"
