param(
    [Parameter(Mandatory = $true)]
    [string]$Path
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
    throw "Tauri signing input is missing"
}
$Resolved = (Resolve-Path -LiteralPath $Path).Path
$Extension = [IO.Path]::GetExtension($Resolved).ToLowerInvariant()
if ($Extension -notin @(".exe", ".dll", ".tmp")) {
    throw "Tauri signing accepts only Windows executables, DLLs, and NSIS temporary uninstallers"
}
if ($Extension -eq ".tmp") {
    # NSIS !uninstfinalize passes its PE uninstaller as nst*.tmp before embedding it.
    $Stream = [IO.File]::OpenRead($Resolved)
    try {
        $Reader = [IO.BinaryReader]::new($Stream)
        if ($Stream.Length -lt 64 -or $Reader.ReadUInt16() -ne 0x5A4D) {
            throw "NSIS temporary uninstaller is not a PE file"
        }
        $Stream.Position = 0x3C
        $PeOffset = $Reader.ReadInt32()
        if ($PeOffset -lt 64 -or $PeOffset -gt ($Stream.Length - 4)) {
            throw "NSIS temporary uninstaller has an invalid PE header offset"
        }
        $Stream.Position = $PeOffset
        if ($Reader.ReadUInt32() -ne 0x00004550) {
            throw "NSIS temporary uninstaller has no PE header"
        }
    } finally {
        $Stream.Dispose()
    }
}
Write-Output "Tauri signing input: $Resolved"
$Signature = Get-AuthenticodeSignature -LiteralPath $Resolved
if ($Signature.Status -eq "Valid") {
    if ($Extension -eq ".dll" -and
        $Signature.TimeStamperCertificate) {
        Write-Output "Tauri DLL already has a valid timestamped publisher signature: $Resolved"
        return
    }
    & (Join-Path $PSScriptRoot "verify-authenticode.ps1") -Path $Resolved
    return
}
if ($Signature.Status -ne "NotSigned") {
    throw "Tauri signing input has an invalid existing signature: $($Signature.Status)"
}

# The pinned Azure action installs this module earlier in the same OIDC-authenticated job.
Import-Module ArtifactSigning -ErrorAction Stop
Invoke-ArtifactSigning `
    -Endpoint "https://eus.codesigning.azure.net/" `
    -CodeSigningAccountName "colossus-code-sign" `
    -CertificateProfileName "colossus" `
    -Files $Resolved `
    -FileDigest "SHA256" `
    -TimestampRfc3161 "http://timestamp.acs.microsoft.com" `
    -TimestampDigest "SHA256"

& (Join-Path $PSScriptRoot "verify-authenticode.ps1") -Path $Resolved
