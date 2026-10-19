param(
    [string]$EvidenceDirectory = '',
    [ValidateRange(10, 300)][int]$TimeoutSeconds = 120
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $IsWindows -or -not [Environment]::UserInteractive) {
    throw 'Computer acceptance requires PowerShell 7 on an unlocked interactive Windows desktop.'
}
$Identity = [Security.Principal.WindowsIdentity]::GetCurrent()
try {
    $Owner = $Identity.User
    $Principal = [Security.Principal.WindowsPrincipal]::new($Identity)
    if ($Principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'Computer acceptance requires an ordinary, non-elevated user token.'
    }
} finally { $Identity.Dispose() }
$Repository = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$ProfileDirectory = [Environment]::GetFolderPath('UserProfile')
if ([string]::IsNullOrWhiteSpace($EvidenceDirectory)) {
    if ([string]::IsNullOrWhiteSpace($ProfileDirectory) -or -not [IO.Path]::IsPathFullyQualified($ProfileDirectory)) {
        throw 'Computer acceptance requires an absolute user profile directory.'
    }
    $EvidenceDirectory = Join-Path $ProfileDirectory ('ColossusComputerTest-' + [Guid]::NewGuid().ToString('N'))
}
$Evidence = [IO.Path]::GetFullPath($EvidenceDirectory, $Repository)
if (Test-Path -LiteralPath $Evidence) { throw 'Computer evidence destination must be fresh.' }
$null = New-Item -ItemType Directory -Path $Evidence
# Only the fresh allocation receives a private DACL; no existing user directory
# or desktop application is modified by the acceptance harness.
$Acl = [Security.AccessControl.DirectorySecurity]::new()
$Acl.SetOwner($Owner)
$Acl.SetAccessRuleProtection($true, $false)
$Inheritance = [Security.AccessControl.InheritanceFlags]::ContainerInherit -bor
    [Security.AccessControl.InheritanceFlags]::ObjectInherit
foreach ($Sid in @($Owner, [Security.Principal.SecurityIdentifier]::new('S-1-5-18'),
        [Security.Principal.SecurityIdentifier]::new('S-1-5-32-544'))) {
    $Acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
        $Sid, [Security.AccessControl.FileSystemRights]::FullControl, $Inheritance,
        [Security.AccessControl.PropagationFlags]::None,
        [Security.AccessControl.AccessControlType]::Allow
    ))
}
Set-Acl -LiteralPath $Evidence -AclObject $Acl
Write-Host "Computer acceptance evidence: $Evidence"
if ($env:GITHUB_OUTPUT) { "evidence=$Evidence" >> $env:GITHUB_OUTPUT }
. (Join-Path $PSScriptRoot 'computer-child-output.ps1')
$Children = [Collections.Generic.List[object]]::new()

function Start-OwnedChild([string]$Name, [string]$Executable, [string[]]$Arguments) {
    $Info = [Diagnostics.ProcessStartInfo]::new($Executable)
    $Info.UseShellExecute = $false
    $Info.CreateNoWindow = $true
    $Info.RedirectStandardOutput = $true
    $Info.RedirectStandardError = $true
    $Info.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
    $Info.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
    $Info.WorkingDirectory = $Repository
    foreach ($Argument in $Arguments) { $Info.ArgumentList.Add($Argument) }
    $Process = [Diagnostics.Process]::Start($Info)
    $Child = [pscustomobject]@{
        Process = $Process
        Stdout = New-ComputerChildOutput $Process.StandardOutput (Join-Path $Evidence "$Name.stdout.log")
        Stderr = New-ComputerChildOutput $Process.StandardError (Join-Path $Evidence "$Name.stderr.log")
    }
    $Children.Add($Child)
    return $Child
}
function Update-ChildLogs {
    foreach ($Child in $Children) {
        Update-ComputerChildOutput $Child.Stdout
        Update-ComputerChildOutput $Child.Stderr
    }
}
function Wait-OwnedExit($Child, [int]$Milliseconds) {
    $Timer = [Diagnostics.Stopwatch]::StartNew()
    do {
        Update-ChildLogs
        if ($Child.Process.WaitForExit(25)) { return $true }
    } while ($Timer.ElapsedMilliseconds -lt $Milliseconds)
    return $false
}
function Complete-ChildLogs($Child) {
    $Deadline = [DateTime]::UtcNow.AddSeconds(2)
    do {
        Update-ChildLogs
        if ($Child.Stdout.Ended -and $Child.Stderr.Ended) { return }
        Start-Sleep -Milliseconds 10
    } while ([DateTime]::UtcNow -lt $Deadline)
    throw 'Computer acceptance child output did not close after exit.'
}
function Wait-Fixture($Child, [string]$Title) {
    $Deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        Update-ChildLogs
        $Child.Process.Refresh()
        if ($Child.Process.HasExited) { throw 'Disposable computer fixture exited before becoming ready.' }
        if ($Child.Process.MainWindowHandle -ne 0 -and $Child.Process.MainWindowTitle -ceq $Title) { return }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $Deadline)
    throw 'Disposable computer fixture did not present its exact owned window.'
}
function Close-OwnedChild([ref]$Reference) {
    $Child = $Reference.Value
    $Reference.Value = $null
    if ($null -eq $Child) { return }
    try {
        if (-not $Child.Process.HasExited) {
            $null = $Child.Process.CloseMainWindow()
            if (-not (Wait-OwnedExit $Child 2000)) {
                # This handle belongs only to a synthetic child started above.
                # A forced fixture/helper teardown is a failed cleanup receipt.
                $Child.Process.Kill($true)
                $null = Wait-OwnedExit $Child 5000
                Complete-ChildLogs $Child
                throw 'Computer acceptance child required forced cleanup.'
            }
        }
        Complete-ChildLogs $Child
    } finally {
        foreach ($Output in @($Child.Stdout, $Child.Stderr)) {
            $Output.Ended = $true
            $Output.Reader.Dispose()
        }
        $Child.Process.Dispose()
    }
}

$Fixture = $null
$Cover = $null
$Acceptance = $null
$Passed = $false
$SourceRevision = $null
$SourceWorktreeClean = $false
$AcceptanceSha256 = $null
$FixtureSha256 = $null
Push-Location $Repository
try {
    $SourceRevision = & git rev-parse HEAD
    if ($LASTEXITCODE -ne 0 -or $SourceRevision -notmatch '^[0-9a-f]{40}$') {
        throw 'Computer acceptance could not bind its source revision.'
    }
    $SourceStatus = & git status --porcelain --untracked-files=all
    if ($LASTEXITCODE -ne 0) { throw 'Computer acceptance could not inspect its source worktree.' }
    $SourceWorktreeClean = @($SourceStatus).Count -eq 0
    & cargo build --locked --manifest-path native/computer/Cargo.toml --features native-test-driver --examples
    if ($LASTEXITCODE -ne 0) { throw 'Computer acceptance fixture build failed.' }
    $Metadata = & cargo metadata --locked --manifest-path native/computer/Cargo.toml --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'Computer acceptance build directory could not be resolved.' }
    $Target = ($Metadata | ConvertFrom-Json).target_directory
    $Examples = Join-Path $Target 'debug/examples'
    $AcceptanceExecutable = Join-Path $Examples 'computer_acceptance.exe'
    $FixtureExecutable = Join-Path $Examples 'computer_window_fixture.exe'
    $AcceptanceSha256 = (Get-FileHash -LiteralPath $AcceptanceExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
    $FixtureSha256 = (Get-FileHash -LiteralPath $FixtureExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
    $Nonce = [Guid]::NewGuid().ToString('N')
    $Title = 'Colossus computer fixture ' + $Nonce
    $Fixture = Start-OwnedChild 'fixture' $FixtureExecutable @('--nonce', $Nonce)
    Wait-Fixture $Fixture $Title
    $Acceptance = Start-OwnedChild 'acceptance' $AcceptanceExecutable @(
        '--fixture-title', $Title, '--output', $Evidence, '--require-cover'
    )
    $Cover = Start-OwnedChild 'cover' $FixtureExecutable @(
        '--nonce', $Nonce, '--cover', '--foreground-recipient', $Acceptance.Process.Id.ToString()
    )
    $CoverTitle = 'Colossus computer cover ' + $Nonce
    Wait-Fixture $Cover $CoverTitle
    Write-Host "Computer acceptance setup: activate the owned cover window '$CoverTitle' if Windows did not place it in front."
    if (-not (Wait-OwnedExit $Acceptance ($TimeoutSeconds * 1000))) {
        throw 'Computer acceptance timed out; native prerequisite or cleanup was not established.'
    }
    Complete-ChildLogs $Acceptance
    if ($Acceptance.Process.ExitCode -ne 0) { throw "Computer acceptance failed ($($Acceptance.Process.ExitCode))." }
    $ReceiptPath = Join-Path $Evidence 'acceptance.json'
    if (-not (Test-Path -LiteralPath $ReceiptPath) -or (Get-Item -LiteralPath $ReceiptPath).Length -gt 64KB) {
        throw 'Computer acceptance did not produce a bounded native receipt.'
    }
    $Receipt = Get-Content -LiteralPath $ReceiptPath -Raw | ConvertFrom-Json
    if ($Receipt.schema_version -ne 1 -or $Receipt.passed -ne $true) {
        throw 'Computer acceptance did not establish every required native check.'
    }
    Close-OwnedChild ([ref]$Acceptance)
    Close-OwnedChild ([ref]$Cover)
    Close-OwnedChild ([ref]$Fixture)
    $Passed = $true
} finally {
    try { Close-OwnedChild ([ref]$Acceptance) } finally {
        try { Close-OwnedChild ([ref]$Cover) } finally {
            try { Close-OwnedChild ([ref]$Fixture) } finally {
                Pop-Location
                $FinalRevision = & git -C $Repository rev-parse HEAD
                $FinalStatus = & git -C $Repository status --porcelain --untracked-files=all
                $SourceWorktreeClean = $SourceWorktreeClean -and $LASTEXITCODE -eq 0 -and
                    $FinalRevision -ceq $SourceRevision -and @($FinalStatus).Count -eq 0
                [IO.File]::WriteAllText((Join-Path $Evidence 'harness.json'),
                    (([ordered]@{schema_version=1; passed=$Passed; platform='windows'; standard_user=$true;
                        source_revision=$SourceRevision; source_dirty=(-not $SourceWorktreeClean);
                        acceptance_sha256=$AcceptanceSha256; fixture_sha256=$FixtureSha256} |
                        ConvertTo-Json) + "`n"), [Text.UTF8Encoding]::new($false))
                if (-not $Passed) {
                    foreach ($Child in $Children) {
                        Write-ComputerChildOutputTail $Child.Stdout
                        Write-ComputerChildOutputTail $Child.Stderr
                    }
                }
            }
        }
    }
}
