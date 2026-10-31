function New-ComposeHttpClient {
    param([string] $CaFile)
    if (-not ('ColossusComposeHttp' -as [type])) {
        Add-Type -TypeDefinition @'
using System;
using System.Net;
using System.Net.Http;
using System.Net.Security;
using System.Security.Cryptography.X509Certificates;
public static class ColossusComposeHttp {
    public static HttpClient Create(string caFile) {
#pragma warning disable SYSLIB0057 // This loader also supports the .NET 6 minimum.
        var root = new X509Certificate2(caFile);
#pragma warning restore SYSLIB0057
        var handler = new HttpClientHandler { AllowAutoRedirect=false, UseProxy=false, CookieContainer=new CookieContainer() };
        handler.ServerCertificateCustomValidationCallback = (request, certificate, chain, errors) => {
            if (certificate == null || (errors & SslPolicyErrors.RemoteCertificateNameMismatch) != 0 ||
                (errors & SslPolicyErrors.RemoteCertificateNotAvailable) != 0) return false;
            using (var verified = new X509Chain()) {
                verified.ChainPolicy.TrustMode = X509ChainTrustMode.CustomRootTrust;
                verified.ChainPolicy.CustomTrustStore.Add(root);
                verified.ChainPolicy.RevocationMode = X509RevocationMode.NoCheck;
                verified.ChainPolicy.ApplicationPolicy.Add(new System.Security.Cryptography.Oid("1.3.6.1.5.5.7.3.1"));
                return verified.Build(certificate);
            }
        };
        return new HttpClient(handler) { Timeout=TimeSpan.FromSeconds(30), MaxResponseContentBufferSize=2*1024*1024 };
    }
}
'@
    }
    return [ColossusComposeHttp]::Create($CaFile)
}

function Invoke-ComposeApi {
    param($Client, [string] $Origin, [string] $Method, [string] $Path, $Body = $null, [int[]] $Statuses = @(200))
    $request = [Net.Http.HttpRequestMessage]::new([Net.Http.HttpMethod]::new($Method),$Origin+$Path)
    try {
        $request.Headers.Add('Origin',$Origin); $request.Headers.Add('X-Colossus-CSRF','1')
        if ($null -ne $Body) { $request.Content = [Net.Http.StringContent]::new(($Body | ConvertTo-Json -Depth 12 -Compress),[Text.Encoding]::UTF8,'application/json') }
        $response = $Client.SendAsync($request).GetAwaiter().GetResult()
        try {
            $status = [int]$response.StatusCode
            if ($status -notin $Statuses) { throw "Deployment API $Method $Path returned $status. No mutation is automatically retried." }
            $text = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
            return @{ status=$status; value=$(if ($text) { $text | ConvertFrom-Json -AsHashtable } else { $null }) }
        } finally { $response.Dispose() }
    } finally { $request.Dispose() }
}

function Initialize-ComposeHostedProject {
    param([string] $Root, $Manifest)
    if ($Manifest.profile -cne 'hosted-demo' -or ($Manifest.ContainsKey('hostedInitialized') -and $Manifest.hostedInitialized)) { return }
    $origin = "https://$($Manifest.hostname):$($Manifest.browserPort)"
    $client = New-ComposeHttpClient (Join-Path $Root 'browser-ca.pem')
    $path = Join-Path $Root 'hosted-bootstrap.json'
    try {
        $password = [IO.File]::ReadAllText((Join-Path $Root 'material/controller/admin-password'))
        $null = Invoke-ComposeApi $client $origin 'POST' '/auth/local' @{ username='admin'; password=$password } @(200,204)
        $identity = (Invoke-ComposeApi $client $origin 'GET' '/api/me').value
        if (-not $identity.user.is_admin) { throw 'Hosted initialization requires the retained bootstrap administrator.' }
        $markerId = if ($Manifest.ContainsKey('restoredFrom')) { $Manifest.restoredFrom.deploymentId } else { $Manifest.deploymentId }
        $marker = "Colossus Compose $markerId"
        $project = Invoke-ComposeApi $client $origin 'GET' '/api/projects/local' $null @(200,404)
        $stage = if (Test-Path -LiteralPath $path) { Get-Content -LiteralPath $path -Raw | ConvertFrom-Json -AsHashtable } else { @{ phase='planned' } }
        if ($project.status -eq 404) {
            if ($stage.phase -cne 'planned' -or $Manifest.ContainsKey('restoredFrom')) { throw 'Project creation outcome is unresolved. Inspect the retained bootstrap receipt; POST will not be retried.' }
            Write-ComposeJson $path @{ phase='create-intent'; deploymentId=$Manifest.deploymentId; adminId=$identity.user.id }
            $project = Invoke-ComposeApi $client $origin 'POST' '/api/projects' @{ id='local'; name='Local Colossus'; description=$marker; parent_project_id=$null }
        }
        if ($project.value.project.description -cne $marker) { throw 'The local project is not owned by this installation.' }
        $members = (Invoke-ComposeApi $client $origin 'GET' '/api/projects/local/members?limit=100').value
        $own = @($members.members | Where-Object user_id -CEQ $identity.user.id)
        if (-not $own.Count) {
            if ($stage.phase -ceq 'membership-intent' -or $Manifest.ContainsKey('restoredFrom')) { throw 'Membership creation outcome is unresolved; it will not be retried.' }
            Write-ComposeJson $path @{ phase='membership-intent'; deploymentId=$Manifest.deploymentId; adminId=$identity.user.id }
            $null = Invoke-ComposeApi $client $origin 'POST' '/api/projects/local/members' @{ user_id=$identity.user.id; role='terminal_operator'; revision=0 }
        } elseif ($own.Count -ne 1 -or $own[0].role -cne 'terminal_operator') { throw 'Retained Hosted membership differs; it will not be overwritten.' }
        $Manifest['hostedInitialized'] = $true
        Write-ComposeJson $path @{ phase='ready'; deploymentId=$Manifest.deploymentId; adminId=$identity.user.id }
        Write-ComposeJson (Join-Path $Root 'manifest.json') $Manifest
    } finally { $client.Dispose() }
}
