param(
    [Parameter(Mandatory=$true)][string]$Executable,
    [Parameter(Mandatory=$true)][string]$Home,
    [switch]$Probe,
    [switch]$Terminate
)

# Developer acceptance only. Process identity comes from an exact staged image
# and a retained native handle. Failure cleanup can terminate only exact images
# in the fresh acceptance stage; it never modifies a token or treats command-line
# sandbox flags as resource enforcement.
$ErrorActionPreference = 'Stop'
try {
    if ($env:OS -ne 'Windows_NT') { throw 'platform' }
    if ($Probe -and $Terminate) { throw 'conflicting operation' }
    $image = [IO.Path]::GetFullPath($Executable)
    if ($Terminate -and [IO.Path]::GetFileName($image) -ne 'colossus-chromium-preview.exe') { throw 'foreign cleanup target' }
    $helper = Join-Path ([IO.Path]::GetDirectoryName($image)) 'colossus-browser-helper.exe'
    $homePath = [IO.Path]::GetFullPath($Home)
    $checkedHome = $homePath
    if (-not $Probe) {
        # Early bootstrap failure may precede private-home creation. Query and
        # cleanup use only the verified stage; do not create or enter this home.
        if ([IO.File]::Exists($checkedHome)) { throw 'home is a file' }
        while (-not [IO.Directory]::Exists($checkedHome)) {
            $parent = [IO.Path]::GetDirectoryName($checkedHome)
            if ([String]::IsNullOrEmpty($parent) -or $parent -eq $checkedHome) { throw 'home ancestry' }
            $checkedHome = $parent
        }
    }
    foreach ($path in @($image, $helper, $checkedHome)) {
        $item = Get-Item -LiteralPath $path -Force
        while ($null -ne $item) {
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'linked path' }
            if ($item -is [IO.FileInfo]) { $item = $item.Directory } else { $item = $item.Parent }
        }
    }
    if (-not [IO.File]::Exists($image) -or -not [IO.File]::Exists($helper) -or ($Probe -and -not [IO.Directory]::Exists($homePath))) { throw 'missing path' }
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Text;

public sealed class ColossusRendererEvidence : IDisposable {
    private IntPtr process;
    private IntPtr token;
    public int Pid { get; private set; }
    public long Creation { get; private set; }
    public string Image { get; private set; }
    public bool Restricted { get; private set; }
    public bool AppContainer { get; private set; }
    public uint Integrity { get; private set; }

    [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr OpenProcess(uint access, bool inherit, int pid);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool GetProcessTimes(IntPtr handle, out long created, out long exited, out long kernel, out long user);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool TerminateProcess(IntPtr handle, uint code);
    [DllImport("kernel32.dll", SetLastError=true)] static extern uint WaitForSingleObject(IntPtr handle, uint timeout);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern bool QueryFullProcessImageNameW(IntPtr handle, uint flags, StringBuilder path, ref uint length);
    [DllImport("advapi32.dll", SetLastError=true)] static extern bool OpenProcessToken(IntPtr process, uint access, out IntPtr token);
    [DllImport("advapi32.dll", SetLastError=true)] static extern bool GetTokenInformation(IntPtr token, int kind, IntPtr data, uint size, out uint length);
    [DllImport("advapi32.dll")] static extern bool IsTokenRestricted(IntPtr token);
    [DllImport("advapi32.dll")] static extern IntPtr GetSidSubAuthorityCount(IntPtr sid);
    [DllImport("advapi32.dll")] static extern IntPtr GetSidSubAuthority(IntPtr sid, uint index);
    [DllImport("advapi32.dll", SetLastError=true)] static extern bool DuplicateTokenEx(IntPtr token, uint access, IntPtr attributes, int level, int type, out IntPtr duplicate);
    [DllImport("advapi32.dll", SetLastError=true)] static extern bool ImpersonateLoggedOnUser(IntPtr token);
    [DllImport("advapi32.dll", SetLastError=true)] static extern bool RevertToSelf();
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)] static extern IntPtr CreateFileW(string path, uint access, uint share, IntPtr attributes, uint disposition, uint flags, IntPtr template);

    static Exception Failure() { return new InvalidOperationException("Native renderer evidence unavailable"); }
    IntPtr Information(int kind) {
        uint size;
        GetTokenInformation(token, kind, IntPtr.Zero, 0, out size);
        if (size == 0 || size > 4096) throw Failure();
        IntPtr buffer = Marshal.AllocHGlobal((int)size);
        if (!GetTokenInformation(token, kind, buffer, size, out size)) {
            Marshal.FreeHGlobal(buffer); throw Failure();
        }
        return buffer;
    }

    public ColossusRendererEvidence(int pid, string expectedImage, bool inspectToken, bool terminate) {
        Pid = pid;
        try {
            process = OpenProcess(terminate ? 0x101001U : 0x1000U, false, pid);
            if (process == IntPtr.Zero) throw Failure();
            long exited, kernel, user, created;
            if (!GetProcessTimes(process, out created, out exited, out kernel, out user)) throw Failure();
            Creation = created;
            var path = new StringBuilder(32768); uint length = 32768;
            if (!QueryFullProcessImageNameW(process, 0, path, ref length)) throw Failure();
            Image = path.ToString();
            if (!String.Equals(Image, expectedImage, StringComparison.OrdinalIgnoreCase)) throw Failure();
            if (inspectToken) {
                if (!OpenProcessToken(process, 0x000A, out token)) throw Failure();
                Restricted = IsTokenRestricted(token);
                IntPtr information = Information(29);
                try { AppContainer = Marshal.ReadInt32(information) != 0; }
                finally { Marshal.FreeHGlobal(information); }
                information = Information(25);
                try {
                    IntPtr sid = Marshal.ReadIntPtr(information);
                    byte count = Marshal.ReadByte(GetSidSubAuthorityCount(sid));
                    if (count == 0) throw Failure();
                    Integrity = unchecked((uint)Marshal.ReadInt32(GetSidSubAuthority(sid, (uint)count - 1)));
                } finally { Marshal.FreeHGlobal(information); }
            }
        } catch { Dispose(); throw; }
    }

    public bool WasCreated(DateTime observed) {
        // Win32_Process timestamps expose microseconds; native FILETIME exposes
        // 100 ns. Compare at the advertised precision before using CIM arguments.
        return observed.ToUniversalTime().ToFileTimeUtc() / 10 == Creation / 10;
    }

    public bool TerminateOwned() {
        // This handle already proved exact staged image and creation identity.
        // A PID lookup at termination could instead target a reused PID.
        if (WaitForSingleObject(process, 0) == 0) return true;
        if (!TerminateProcess(process, 1)) return false;
        return WaitForSingleObject(process, 200) == 0;
    }

    public bool FileReadDenied(string fixture) {
        IntPtr duplicate;
        if (token == IntPtr.Zero || !DuplicateTokenEx(token, 0x000C, IntPtr.Zero, 2, 2, out duplicate)) throw Failure();
        try {
            if (!ImpersonateLoggedOnUser(duplicate)) throw Failure();
            bool denied = false;
            try {
                // A real kernel file open under the actual renderer token. No
                // fixture bytes are read or copied into acceptance output.
                IntPtr file = CreateFileW(fixture, 0x80000000, 7, IntPtr.Zero, 3, 0x80, IntPtr.Zero);
                int error = Marshal.GetLastWin32Error();
                if (file == new IntPtr(-1)) denied = error == 5;
                else CloseHandle(file);
            } finally {
                // Never continue in PowerShell under an impersonated token.
                if (!RevertToSelf()) Environment.FailFast("Renderer impersonation could not be reverted");
            }
            return denied;
        } finally { CloseHandle(duplicate); }
    }

    public void Dispose() {
        if (token != IntPtr.Zero) { CloseHandle(token); token = IntPtr.Zero; }
        if (process != IntPtr.Zero) { CloseHandle(process); process = IntPtr.Zero; }
    }
}
'@
    $handles = [Collections.Generic.List[ColossusRendererEvidence]]::new()
    $owned = @()
    $rendererCount = 0
    $restricted = $true
    $denied = $true
    $safeFlags = $true
    $noDebug = $true
    $fixtureDirectory = $null
    try {
        if ($Probe) {
            $fixtureDirectory = Join-Path $homePath ('renderer-denial-' + [Guid]::NewGuid().ToString('N'))
            [IO.Directory]::CreateDirectory($fixtureDirectory) | Out-Null
            $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User
            $acl = [Security.AccessControl.DirectorySecurity]::new()
            $acl.SetOwner($sid)
            $acl.SetAccessRuleProtection($true, $false)
            $rule = [Security.AccessControl.FileSystemAccessRule]::new($sid, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow')
            $acl.AddAccessRule($rule)
            Set-Acl -LiteralPath $fixtureDirectory -AclObject $acl
            $fixture = Join-Path $fixtureDirectory 'owner-only.txt'
            [IO.File]::WriteAllText($fixture, 'Colossus renderer denial acceptance fixture')
        }
        $processes = @(Get-CimInstance Win32_Process -ErrorAction Stop | Where-Object {
            $null -ne $_.ExecutablePath -and (
                [String]::Equals($_.ExecutablePath, $image, [StringComparison]::OrdinalIgnoreCase) -or
                [String]::Equals($_.ExecutablePath, $helper, [StringComparison]::OrdinalIgnoreCase)
            )
        })
        if ($processes.Count -gt 64) { throw 'process bounds' }
        foreach ($candidate in $processes) {
            $line = [string]$candidate.CommandLine
            if ($line.Length -gt 32768) { throw 'command bounds' }
            $renderer = $line -match '(?:^|\s)--type=renderer(?:\s|$)'
            $expectedImage = [string]$candidate.ExecutablePath
            if (-not ([String]::Equals($expectedImage, $image, [StringComparison]::OrdinalIgnoreCase) -or
                [String]::Equals($expectedImage, $helper, [StringComparison]::OrdinalIgnoreCase))) { throw 'foreign image' }
            $facts = [ColossusRendererEvidence]::new([int]$candidate.ProcessId, $expectedImage, ($Probe -and $renderer), [bool]$Terminate)
            $handles.Add($facts)
            if (-not $facts.WasCreated([DateTime]$candidate.CreationDate)) { throw 'process identity changed' }
            $safeFlags = $safeFlags -and ($line -notmatch '(?:^|\s)--(?:no-sandbox|disable-(?:gpu|renderer|setuid)-sandbox)(?:=|\s|$)')
            $noDebug = $noDebug -and ($line -notmatch '(?:^|\s)--remote-debugging-(?:port|pipe)(?:=|\s|$)')
            $kind = if ($line -match '(?:^|\s)--type=([a-z-]+)(?:\s|$)') { $Matches[1] } else { 'browser' }
            $owned += [ordered]@{
                pid = $facts.Pid
                creationTime = [DateTime]::FromFileTimeUtc($facts.Creation).ToString('o')
                creationFileTime = $facts.Creation.ToString()
                executablePath = $facts.Image
                # Retain only the closed process type, never complete command
                # arguments, pipe handles, credentials, or page destinations.
                commandLine = "--type=$kind"
            }
            if ($renderer) {
                $rendererCount++
                if ($Probe) {
                    $restricted = $restricted -and (($facts.Restricted -or $facts.AppContainer) -and $facts.Integrity -le 0x1000)
                    $denied = $denied -and $facts.FileReadDenied($fixture)
                }
            }
        }
        $terminationSucceeded = $true
        if ($Terminate) {
            foreach ($handle in $handles) {
                $settled = $handle.TerminateOwned()
                $terminationSucceeded = $terminationSucceeded -and $settled
            }
            $survivors = @(Get-CimInstance Win32_Process -ErrorAction Stop | Where-Object {
                $null -ne $_.ExecutablePath -and (
                    [String]::Equals($_.ExecutablePath, $image, [StringComparison]::OrdinalIgnoreCase) -or
                    [String]::Equals($_.ExecutablePath, $helper, [StringComparison]::OrdinalIgnoreCase)
                )
            })
            $terminationSucceeded = $terminationSucceeded -and $survivors.Count -eq 0
        }
        $passed = $safeFlags -and $noDebug -and $terminationSucceeded -and ((-not $Probe) -or ($rendererCount -gt 0 -and $restricted -and $denied))
        $result = [ordered]@{
            processQuerySucceeded = $true
            owned = @($owned)
            rendererCount = $rendererCount
            restrictedRendererTokens = ($Probe -and $rendererCount -gt 0 -and $restricted)
            rendererFileAccessDenied = ($Probe -and $rendererCount -gt 0 -and $denied)
            unsafeSandboxFlagsAbsent = $safeFlags
            remoteDebuggingPortAbsent = $noDebug
            terminatedOwned = ($Terminate -and $terminationSucceeded)
            passed = [bool]$passed
        }
        $json = ConvertTo-Json -InputObject $result -Depth 5 -Compress
        if ([Text.Encoding]::UTF8.GetByteCount($json) -gt 65536) { throw 'output bounds' }
        [Console]::Out.WriteLine($json)
        if (-not $passed) { exit 1 }
    } finally {
        foreach ($handle in $handles) { $handle.Dispose() }
        if ($null -ne $fixtureDirectory) { Remove-Item -LiteralPath $fixtureDirectory -Recurse -Force }
    }
} catch {
    [Console]::Out.WriteLine('{"processQuerySucceeded":false,"owned":[],"passed":false}')
    [Console]::Error.WriteLine('Windows Chromium process evidence unavailable; native acceptance did not pass.')
    exit 1
}
