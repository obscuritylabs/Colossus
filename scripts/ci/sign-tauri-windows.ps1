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
if ([IO.Path]::GetExtension($Resolved) -notin @(".exe", ".dll")) {
    throw "Tauri signing accepts only Windows executables and DLLs"
}
$Signature = Get-AuthenticodeSignature -LiteralPath $Resolved
if ($Signature.Status -eq "Valid") {
    if ([IO.Path]::GetExtension($Resolved) -eq ".dll" -and
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
