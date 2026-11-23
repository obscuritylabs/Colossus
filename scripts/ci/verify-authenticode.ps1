param(
    [Parameter(Mandatory = $true)]
    [string]$Path,
    [switch]$RequireEmbeddedSignature
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
    throw 'Authenticode verification input is missing'
}
if ($RequireEmbeddedSignature) {
    $image = Get-Item -LiteralPath $Path
    if (-not $IsWindows -or $image.Extension -notin @('.exe', '.dll')) {
        throw 'Embedded Authenticode verification requires a Windows browser executable or DLL'
    }
    if (-not ('Colossus.Release.EmbeddedAuthenticode' -as [type])) {
        Add-Type -LiteralPath (Join-Path $PSScriptRoot 'embedded-authenticode.cs')
    }
    [Colossus.Release.EmbeddedAuthenticode]::Verify($image.FullName)
} else {
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
}
Write-Output "Verified Obscurity Labs LLC Authenticode signature: $Path"
