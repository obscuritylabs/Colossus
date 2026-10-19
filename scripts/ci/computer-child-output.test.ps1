$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'computer-child-output.ps1')
$Root = Join-Path ([IO.Path]::GetTempPath()) ('ColossusComputerOutputTest-' + [Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $Root
$Child = $null
try {
    # A synthetic console child fills both pipes beyond their capacity, writes
    # multibyte UTF-8 diagnostics, then fails. No desktop or native helper runs.
    $Source = @'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$Payload = ([string][char]0x4E16 + [char]::ConvertFromUtf32(0x1F600)) * 70000
[Console]::Out.Write($Payload)
[Console]::Out.WriteLine('stdout-final')
[Console]::Error.Write($Payload)
[Console]::Error.WriteLine('stderr-final')
exit 7
'@
    $Info = [Diagnostics.ProcessStartInfo]::new([Environment]::ProcessPath)
    $Info.UseShellExecute = $false
    $Info.CreateNoWindow = $true
    $Info.RedirectStandardOutput = $true
    $Info.RedirectStandardError = $true
    $Info.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
    $Info.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
    foreach ($Argument in @('-NoProfile', '-NonInteractive', '-EncodedCommand',
            [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($Source)))) {
        $Info.ArgumentList.Add($Argument)
    }
    $Child = [Diagnostics.Process]::Start($Info)
    $Stdout = New-ComputerChildOutput $Child.StandardOutput (Join-Path $Root 'stdout.log')
    $Stderr = New-ComputerChildOutput $Child.StandardError (Join-Path $Root 'stderr.log')
    $Deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        Update-ComputerChildOutput $Stdout
        Update-ComputerChildOutput $Stderr
        if ($Child.HasExited -and $Stdout.Ended -and $Stderr.Ended) { break }
        Start-Sleep -Milliseconds 5
    } while ([DateTime]::UtcNow -lt $Deadline)
    if (-not $Child.HasExited -or $Child.ExitCode -ne 7 -or -not $Stdout.Ended -or -not $Stderr.Ended) {
        throw 'Bounded diagnostic collection deadlocked or lost child failure.'
    }
    foreach ($Case in @(@($Stdout, 'stdout-final'), @($Stderr, 'stderr-final'))) {
        $Output = $Case[0]
        $Text = [IO.File]::ReadAllText($Output.Path)
        if ((Get-Item -LiteralPath $Output.Path).Length -gt 131200 -or
            -not $Output.Truncated -or -not $Text.StartsWith('[earlier output truncated;') -or
            -not $Text.TrimEnd().EndsWith($Case[1]) -or $Text.Contains([char]0xFFFD)) {
            throw 'Bounded UTF-8 log lost its final diagnostic or exceeded its byte bound.'
        }
    }
    Write-Host 'PASS bounded stdout/stderr retain UTF-8 failure tails without pipe deadlock'
} finally {
    if ($null -ne $Child) {
        if (-not $Child.HasExited) { $Child.Kill($true); $null = $Child.WaitForExit(2000) }
        $Child.Dispose()
    }
    foreach ($Name in @('stdout.log', 'stderr.log')) { [IO.File]::Delete((Join-Path $Root $Name)) }
    [IO.Directory]::Delete($Root)
}
