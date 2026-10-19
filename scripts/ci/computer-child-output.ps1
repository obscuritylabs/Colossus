# Bounded diagnostics for synthetic acceptance children. Drain stdout and stderr
# independently so a full pipe cannot deadlock readiness, exit or cleanup waits.
function New-ComputerChildOutput([IO.TextReader]$Reader, [string]$Path) {
    $Buffer = [char[]]::new(2048)
    return [pscustomobject]@{
        Reader = $Reader; Path = $Path; Buffer = $Buffer
        Pending = $Reader.ReadAsync($Buffer, 0, $Buffer.Length)
        Text = [Text.StringBuilder]::new(); Ended = $false; Truncated = $false
    }
}
function Update-ComputerChildOutput($Output) {
    $Changed = $false
    # A bounded number of chunks per poll keeps the other pipe and process
    # deadline responsive even if one child continuously writes diagnostics.
    for ($Chunk = 0; $Chunk -lt 8 -and -not $Output.Ended -and $Output.Pending.IsCompleted; $Chunk++) {
        $Count = $Output.Pending.GetAwaiter().GetResult()
        if ($Count -eq 0) { $Output.Ended = $true; break }
        $null = $Output.Text.Append($Output.Buffer, 0, $Count)
        if ($Output.Text.Length -gt 32768) {
            $Remove = $Output.Text.Length - 32768
            if ([char]::IsLowSurrogate($Output.Text[$Remove]) -and
                [char]::IsHighSurrogate($Output.Text[$Remove - 1])) { $Remove++ }
            $null = $Output.Text.Remove(0, $Remove)
            $Output.Truncated = $true
        }
        $Changed = $true
        $Output.Pending = $Output.Reader.ReadAsync($Output.Buffer, 0, $Output.Buffer.Length)
    }
    if ($Changed -or -not (Test-Path -LiteralPath $Output.Path)) {
        $Header = if ($Output.Truncated) { "[earlier output truncated; retaining final 32768 characters]`n" } else { '' }
        [IO.File]::WriteAllText($Output.Path, $Header + $Output.Text.ToString(), [Text.UTF8Encoding]::new($false))
    }
}
function Write-ComputerChildOutputTail($Output) {
    if ((Test-Path -LiteralPath $Output.Path) -and (Get-Item -LiteralPath $Output.Path).Length -gt 0) {
        Write-Host ("--- " + [IO.Path]::GetFileName($Output.Path) + ' (final 16 lines) ---')
        foreach ($Line in Get-Content -LiteralPath $Output.Path -Tail 16) {
            Write-Host $Line.Substring(0, [Math]::Min($Line.Length, 1024))
        }
    }
}
