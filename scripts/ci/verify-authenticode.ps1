param(
    [Parameter(Mandatory = $true)]
    [string]$Path
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
    throw 'Authenticode verification input is missing'
}
$signature = Get-AuthenticodeSignature -LiteralPath $Path
if ($signature.Status -ne 'Valid') {
    throw "Authenticode verification failed: $($signature.Status)"
}
if ($signature.SignerCertificate.Subject -notmatch 'CN=Obscurity Labs LLC(?:,|$)') {
    throw 'Authenticode publisher is not Obscurity Labs LLC'
}
if (-not $signature.TimeStamperCertificate) {
    throw 'Authenticode timestamp is missing'
}
Write-Output "Verified Obscurity Labs LLC Authenticode signature: $Path"
