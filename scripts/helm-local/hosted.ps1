. (Join-Path $PSScriptRoot 'hosted-material.ps1')

function Get-HelmLocalCanonicalJson {
    param($Value)
    # Ordered keys make receipt hashes independent of Hashtable enumeration.
    function ConvertTo-OrderedValue($Item) {
        if ($Item -is [Collections.IDictionary]) {
            $ordered = [ordered]@{}
            foreach ($key in @($Item.Keys | Sort-Object)) { $ordered[$key] = ConvertTo-OrderedValue $Item[$key] }
            return $ordered
        }
        if ($Item -is [Collections.IEnumerable] -and $Item -isnot [string]) {
            return ,@($Item | ForEach-Object { ConvertTo-OrderedValue $_ })
        }
        return $Item
    }
    return (ConvertTo-OrderedValue $Value | ConvertTo-Json -Depth 70 -Compress)
}

function Assert-HelmLocalDesiredObject {
    param($Desired, $Actual)
    if ($Desired -is [Collections.IDictionary]) {
        if ($Actual -isnot [Collections.IDictionary]) { throw 'Owned resource shape changed.' }
        foreach ($key in $Desired.Keys) {
            if (-not $Actual.Contains($key)) { throw 'Owned resource field is missing.' }
            Assert-HelmLocalDesiredObject $Desired[$key] $Actual[$key]
        }
    } elseif ($Desired -is [Collections.IList]) {
        if ($Actual -isnot [Collections.IList] -or $Desired.Count -ne $Actual.Count) { throw 'Owned resource array changed.' }
        for ($i = 0; $i -lt $Desired.Count; $i++) { Assert-HelmLocalDesiredObject $Desired[$i] $Actual[$i] }
    } elseif ([string]$Desired -cne [string]$Actual) { throw 'Owned resource configuration changed; refusing to overwrite it.' }
}

function Ensure-HelmLocalHostedObject {
    param($Tools, [string] $Root, [hashtable] $Manifest, [hashtable] $Object)
    $hosted = $Manifest.hosted
    if (-not $Object.metadata.ContainsKey('labels')) { $Object.metadata.labels = @{} }
    $Object.metadata.labels['colossus.ai/local-owner'] = $hosted.ownerId
    $Object.metadata.labels['app.kubernetes.io/managed-by'] = 'colossus-helm-local'
    $namespace = if ($Object.metadata.ContainsKey('namespace')) { [string]$Object.metadata.namespace } else { '' }
    $key = "$($Object.kind)/$namespace/$($Object.metadata.name)"
    $json = Get-HelmLocalCanonicalJson $Object
    $digest = Get-HelmLocalSha256 ([Text.Encoding]::UTF8.GetBytes($json))
    $entry = @($hosted.resources | Where-Object key -CEQ $key)
    if ($entry.Count -gt 1) { throw 'Duplicate Hosted ownership receipt.' }
    $previous = $null
    if ($entry.Count -eq 1 -and $entry[0].desiredSha256 -cne $digest) {
        $oldPath = Join-Path $hosted.privateAllocation.path ('resource-' + $entry[0].desiredSha256 + '.json')
        if (-not (Test-Path -LiteralPath $oldPath -PathType Leaf) -or (Get-HelmLocalSha256 ([IO.File]::ReadAllBytes($oldPath))) -cne $entry[0].desiredSha256) { throw 'Previous Hosted resource input is missing or changed.' }
        $previous = [IO.File]::ReadAllText($oldPath) | ConvertFrom-Json -AsHashtable
        $allowed = [IO.File]::ReadAllText($oldPath) | ConvertFrom-Json -AsHashtable
        # A selected application image upgrade may advance the anchor binary.
        # Database/TLS/key/PVC/config changes require a separately planned upgrade.
        if ($Object.kind -cne 'Deployment' -or $Object.metadata.name -cne 'colossus-chat-anchor') { throw "Retained Hosted resource input changed: $key." }
        $allowed.spec.template.spec.containers[0].image = $Object.spec.template.spec.containers[0].image
        if ((Get-HelmLocalCanonicalJson $allowed) -cne $json) { throw "Hosted infrastructure change requires a deliberate upgrade: $key." }
    }
    $arguments = @('get', $Object.kind, $Object.metadata.name, '--ignore-not-found', '-o', 'json')
    if ($namespace) { $arguments += @('-n', $namespace) }
    $raw = (Invoke-HelmLocalKubectl $Tools $Root $Manifest.clusterName $arguments).Trim()
    if ($raw) {
        if ($entry.Count -ne 1) { throw "An unowned resource already exists: $key." }
        $actual = $raw | ConvertFrom-Json -AsHashtable
        if ($actual.metadata.labels['colossus.ai/local-owner'] -cne $hosted.ownerId -or
            ($entry[0].uid -and $entry[0].uid -cne $actual.metadata.uid)) { throw "Hosted resource identity changed: $key." }
        if ($null -ne $previous) {
            $pendingMatches = $entry[0].ContainsKey('pendingSha256') -and $entry[0].pendingSha256 -ceq $digest
            $alreadyUpdated = $false
            if ($pendingMatches) {
                try { Assert-HelmLocalDesiredObject $Object $actual; $alreadyUpdated = $true } catch { $alreadyUpdated = $false }
            }
            if (-not $alreadyUpdated) {
                Assert-HelmLocalDesiredObject $previous $actual
                $entry[0].pendingSha256 = $digest
                Write-HelmLocalJson (Join-Path $Root 'manifest.json') $Manifest
                $patch = @(
                    @{ op = 'test'; path = '/metadata/uid'; value = $actual.metadata.uid },
                    @{ op = 'test'; path = '/metadata/resourceVersion'; value = $actual.metadata.resourceVersion },
                    @{ op = 'test'; path = '/spec/template/spec/containers/0/image'; value = $previous.spec.template.spec.containers[0].image },
                    @{ op = 'replace'; path = '/spec/template/spec/containers/0/image'; value = $Object.spec.template.spec.containers[0].image }
                )
                $patchPath = Join-Path $hosted.privateAllocation.path ('image-patch-' + [Guid]::NewGuid().ToString('N') + '.json')
                Write-HelmLocalPrivateFile $patchPath (Get-HelmLocalCanonicalJson $patch)
                $actual = Invoke-HelmLocalKubectl $Tools $Root $Manifest.clusterName @('-n', $namespace, 'patch', 'deployment', $Object.metadata.name, '--type=json', '--patch-file', $patchPath, '-o', 'json') | ConvertFrom-Json -AsHashtable
            }
        }
        Assert-HelmLocalDesiredObject $Object $actual
    } else {
        if ($entry.Count -eq 1 -and $entry[0].uid) { throw "A retained Hosted resource is missing: $key. Refusing replacement of retained state." }
        if ($entry.Count -eq 0) {
            $hosted.resources += @{ key = $key; kind = $Object.kind; namespace = $namespace; name = $Object.metadata.name; uid = ''; desiredSha256 = $digest }
            Write-HelmLocalJson (Join-Path $Root 'manifest.json') $Manifest
            $entry = @($hosted.resources | Where-Object key -CEQ $key)
        }
        # Intent is durable before create. A retry may recover only this owner's
        # exact desired resource after a create succeeded but its response was lost.
        $createPath = Join-Path $hosted.privateAllocation.path ('resource-' + $digest + '.json')
        if (-not (Test-Path -LiteralPath $createPath)) { Write-HelmLocalPrivateFile $createPath $json }
        if ((Get-HelmLocalSha256 ([IO.File]::ReadAllBytes($createPath))) -cne $digest) { throw 'Retained private resource manifest changed.' }
        $actual = Invoke-HelmLocalKubectl $Tools $Root $Manifest.clusterName @('create', '-f', $createPath, '-o', 'json') | ConvertFrom-Json -AsHashtable
        Assert-HelmLocalDesiredObject $Object $actual
    }
    $path = Join-Path $hosted.privateAllocation.path ('resource-' + $digest + '.json')
    if (-not (Test-Path -LiteralPath $path)) { Write-HelmLocalPrivateFile $path $json }
    if ((Get-HelmLocalSha256 ([IO.File]::ReadAllBytes($path))) -cne $digest) { throw 'Retained private resource manifest changed.' }
    $entry[0].uid = [string]$actual.metadata.uid
    $entry[0].desiredSha256 = $digest
    if ($entry[0].ContainsKey('pendingSha256')) { $entry[0].Remove('pendingSha256') }
    Write-HelmLocalJson (Join-Path $Root 'manifest.json') $Manifest
}

function New-HelmLocalControlledProject {
    . (Join-Path $PSScriptRoot '../../deploy/control-plane/controlled-project.ps1')
    return New-ColossusControlledProject
}

function New-HelmLocalDatabaseObjects {
    param([string] $Namespace, [string] $Name, [string] $Database, [string] $User, [string] $Size, [string] $Image)
    $selector = @{ 'app.kubernetes.io/name' = $Name }
    $memoryRequest = if ($Name -ceq 'colossus-controller-postgres') { '512Mi' } else { '256Mi' }
    $memoryLimit = if ($Name -ceq 'colossus-controller-postgres') { '1Gi' } else { '512Mi' }
    $config = @"
listen_addresses='*'
port=5432
ssl=on
ssl_cert_file='/tls/tls.crt'
ssl_key_file='/tls/tls.key'
ssl_min_protocol_version='TLSv1.2'
password_encryption='scram-sha-256'
hba_file='/etc/postgresql/pg_hba.conf'
max_connections=100
shared_buffers='64MB'
logging_collector=off
log_statement='none'
log_min_error_statement='panic'
"@
    $hba = "local all all trust`nhostnossl all all 0.0.0.0/0 reject`nhostnossl all all ::/0 reject`nhostssl $Database $User 0.0.0.0/0 scram-sha-256`nhostssl $Database $User ::/0 scram-sha-256`nhost all all 0.0.0.0/0 reject`nhost all all ::/0 reject`n"
    $init = @'
#!/bin/sh
set -eu
psql -X -q -v ON_ERROR_STOP=1 --username postgres --dbname postgres <<'SQL'
\getenv app_password APP_PASSWORD
SELECT format('CREATE ROLE __USER__ LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD %L', :'app_password') \gexec
CREATE DATABASE __DATABASE__ OWNER __USER__;
REVOKE ALL ON DATABASE __DATABASE__ FROM PUBLIC;
GRANT CONNECT,TEMPORARY ON DATABASE __DATABASE__ TO __USER__;
\connect __DATABASE__
REVOKE ALL ON SCHEMA public FROM PUBLIC;
SQL
'@
    $init = $init.Replace('__USER__', $User).Replace('__DATABASE__', $Database)
    $security = @{ runAsNonRoot = $true; runAsUser = 999; runAsGroup = 999; readOnlyRootFilesystem = $true; allowPrivilegeEscalation = $false; capabilities = @{ drop = @('ALL') } }
    return @(
        @{ apiVersion = 'v1'; kind = 'ConfigMap'; metadata = @{ name = "$Name-config"; namespace = $Namespace }; data = @{ 'postgresql.conf' = $config; 'pg_hba.conf' = $hba; '10-app.sh' = $init } },
        @{ apiVersion = 'v1'; kind = 'PersistentVolumeClaim'; metadata = @{ name = "$Name-data"; namespace = $Namespace }; spec = @{ accessModes = @('ReadWriteOnce'); storageClassName = 'standard'; resources = @{ requests = @{ storage = $Size } } } },
        @{ apiVersion = 'v1'; kind = 'Service'; metadata = @{ name = $Name; namespace = $Namespace }; spec = @{ selector = $selector; ports = @(@{ name = 'postgres'; port = 5432; targetPort = 5432 }) } },
        @{ apiVersion = 'apps/v1'; kind = 'StatefulSet'; metadata = @{ name = $Name; namespace = $Namespace }; spec = @{
            replicas = 1; serviceName = $Name; selector = @{ matchLabels = $selector }
            template = @{ metadata = @{ labels = $selector }; spec = @{
                automountServiceAccountToken = $false; enableServiceLinks = $false; terminationGracePeriodSeconds = 60
                securityContext = @{ runAsNonRoot = $true; runAsUser = 999; runAsGroup = 999; fsGroup = 999; seccompProfile = @{ type = 'RuntimeDefault' } }
                initContainers = @(@{ name = 'private-storage'; image = $Image; imagePullPolicy = 'IfNotPresent'; command = @('/bin/sh', '-ec', 'mkdir -p /data/pgdata; chmod 0700 /data/pgdata; chown 999:999 /data/pgdata; install -o 999 -g 999 -m 0600 /input/tls.key /tls/tls.key; install -o 999 -g 999 -m 0644 /input/tls.crt /tls/tls.crt')
                    securityContext = @{ runAsNonRoot = $false; runAsUser = 0; runAsGroup = 0; readOnlyRootFilesystem = $true; allowPrivilegeEscalation = $false; capabilities = @{ drop = @('ALL'); add = @('CHOWN', 'FOWNER', 'DAC_OVERRIDE') } }
                    resources = @{ requests = @{ cpu = '10m'; memory = '16Mi' }; limits = @{ cpu = '100m'; memory = '64Mi' } }
                    volumeMounts = @(@{ name = 'data'; mountPath = '/data' }, @{ name = 'tls-input'; mountPath = '/input'; readOnly = $true }, @{ name = 'tls'; mountPath = '/tls' }) })
                containers = @(@{ name = 'postgres'; image = $Image; imagePullPolicy = 'IfNotPresent'; args = @('postgres', '-c', 'config_file=/etc/postgresql/postgresql.conf'); securityContext = $security
                    env = @(@{ name = 'PGDATA'; value = '/var/lib/postgresql/data/pgdata' }, @{ name = 'POSTGRES_USER'; value = 'postgres' }, @{ name = 'POSTGRES_DB'; value = 'postgres' },
                        @{ name = 'POSTGRES_PASSWORD'; valueFrom = @{ secretKeyRef = @{ name = "$Name-auth"; key = 'admin-password' } } }, @{ name = 'APP_PASSWORD'; valueFrom = @{ secretKeyRef = @{ name = "$Name-auth"; key = 'app-password' } } })
                    ports = @(@{ name = 'postgres'; containerPort = 5432 }); resources = @{ requests = @{ cpu = '100m'; memory = $memoryRequest }; limits = @{ cpu = '1'; memory = $memoryLimit } }
                    readinessProbe = @{ exec = @{ command = @('pg_isready', '-U', 'postgres', '-d', 'postgres') }; periodSeconds = 5 }
                    volumeMounts = @(@{ name = 'data'; mountPath = '/var/lib/postgresql/data' }, @{ name = 'config'; mountPath = '/etc/postgresql'; readOnly = $true }, @{ name = 'config'; mountPath = '/docker-entrypoint-initdb.d'; readOnly = $true }, @{ name = 'tls'; mountPath = '/tls'; readOnly = $true }, @{ name = 'run'; mountPath = '/var/run/postgresql' }, @{ name = 'tmp'; mountPath = '/tmp' }) })
                volumes = @(@{ name = 'data'; persistentVolumeClaim = @{ claimName = "$Name-data" } }, @{ name = 'config'; configMap = @{ name = "$Name-config"; defaultMode = 365 } }, @{ name = 'tls-input'; secret = @{ secretName = "$Name-tls"; defaultMode = 256 } },
                    @{ name = 'tls'; emptyDir = @{ medium = 'Memory'; sizeLimit = '1Mi' } }, @{ name = 'run'; emptyDir = @{ medium = 'Memory'; sizeLimit = '4Mi' } }, @{ name = 'tmp'; emptyDir = @{ sizeLimit = '32Mi' } })
            } }
        } }
    )
}

function New-HelmLocalAnchorObjects {
    param([string] $Namespace, [string] $ServerImage, [string] $EnvoyImage)
    $name = 'colossus-chat-anchor'; $dns = "$name.$Namespace.svc"
    $anchor = @{ bind = '127.0.0.1:8080'; trustedTlsIngress = $false; database = @{ connectionVariable = 'COLOSSUS_ANCHOR_DATABASE'; schema = 'colossus_chat_anchors'; tls = @{ kind = 'custom_ca'; caPemPath = '/etc/colossus/database/ca.pem' }; maxConnections = 4; connectionTimeoutMs = 5000; statementTimeoutMs = 5000 }; bearerVariable = 'COLOSSUS_ANCHOR_TOKEN'; storageKeyVariable = 'COLOSSUS_ANCHOR_STORAGE_KEY' }
    $envoy = @{ static_resources = @{
        listeners = @(@{ name = 'anchor_https'; address = @{ socket_address = @{ address = '0.0.0.0'; port_value = 8443 } }; filter_chains = @(@{
            transport_socket = @{ name = 'envoy.transport_sockets.tls'; typed_config = @{ '@type' = 'type.googleapis.com/envoy.extensions.transport_sockets.tls.v3.DownstreamTlsContext'; common_tls_context = @{ tls_params = @{ tls_minimum_protocol_version = 'TLSv1_2' }; tls_certificates = @(@{ certificate_chain = @{ filename = '/etc/anchor/tls/tls.crt' }; private_key = @{ filename = '/etc/anchor/tls/tls.key' } }) } } }
            filters = @(@{ name = 'envoy.filters.network.http_connection_manager'; typed_config = @{ '@type' = 'type.googleapis.com/envoy.extensions.filters.network.http_connection_manager.v3.HttpConnectionManager'; stat_prefix = 'anchor_https'; route_config = @{ name = 'anchor_only'; virtual_hosts = @(@{ name = 'anchor'; domains = @($dns, "$($dns):443", "$dns.cluster.local", "$dns.cluster.local:443"); routes = @(@{ match = @{ prefix = '/v1/anchors/' }; route = @{ cluster = 'anchor_loopback'; timeout = '5s' } }) }) }; http_filters = @(@{ name = 'envoy.filters.http.router'; typed_config = @{ '@type' = 'type.googleapis.com/envoy.extensions.filters.http.router.v3.Router' } }) } })
        }) })
        clusters = @(@{ name = 'anchor_loopback'; connect_timeout = '1s'; type = 'STATIC'; load_assignment = @{ cluster_name = 'anchor_loopback'; endpoints = @(@{ lb_endpoints = @(@{ endpoint = @{ address = @{ socket_address = @{ address = '127.0.0.1'; port_value = 8080 } } } }) }) } })
    } }
    $selector = @{ 'app.kubernetes.io/name' = $name }
    $security = @{ runAsNonRoot = $true; readOnlyRootFilesystem = $true; allowPrivilegeEscalation = $false; capabilities = @{ drop = @('ALL') } }
    $env = @(@{ name = 'COLOSSUS_ANCHOR_DATABASE'; valueFrom = @{ secretKeyRef = @{ name = 'colossus-anchor-client'; key = 'database' } } }, @{ name = 'COLOSSUS_ANCHOR_TOKEN'; valueFrom = @{ secretKeyRef = @{ name = 'colossus-anchor-state'; key = 'bearer' } } }, @{ name = 'COLOSSUS_ANCHOR_STORAGE_KEY'; valueFrom = @{ secretKeyRef = @{ name = 'colossus-anchor-state'; key = 'storage-key' } } })
    return @(
        @{ apiVersion = 'v1'; kind = 'ConfigMap'; metadata = @{ name = $name; namespace = $Namespace }; data = @{ 'anchor.json' = Get-HelmLocalCanonicalJson $anchor; 'envoy.json' = Get-HelmLocalCanonicalJson $envoy } },
        @{ apiVersion = 'v1'; kind = 'Service'; metadata = @{ name = $name; namespace = $Namespace }; spec = @{ selector = $selector; ports = @(@{ name = 'https'; port = 443; targetPort = 'https' }) } },
        @{ apiVersion = 'apps/v1'; kind = 'Deployment'; metadata = @{ name = $name; namespace = $Namespace }; spec = @{ replicas = 1; selector = @{ matchLabels = $selector }; template = @{ metadata = @{ labels = $selector }; spec = @{
            automountServiceAccountToken = $false; enableServiceLinks = $false; terminationGracePeriodSeconds = 30; securityContext = @{ runAsNonRoot = $true; fsGroup = 10002; seccompProfile = @{ type = 'RuntimeDefault' } }
            containers = @(
                @{ name = 'anchor'; image = $ServerImage; imagePullPolicy = 'IfNotPresent'; command = @('/usr/local/bin/colossus-chat-anchor'); args = @('/etc/anchor/config/anchor.json'); env = $env
                    securityContext = ($security + @{ runAsUser = 10001; runAsGroup = 10001 }); resources = @{ requests = @{ cpu = '50m'; memory = '64Mi' }; limits = @{ cpu = '500m'; memory = '256Mi' } }
                    readinessProbe = @{ exec = @{ command = @('/bin/bash', '-ec', 'exec 3<>/dev/tcp/127.0.0.1/8080') }; periodSeconds = 5 }
                    volumeMounts = @(@{ name = 'config'; mountPath = '/etc/anchor/config'; readOnly = $true }, @{ name = 'db-ca'; mountPath = '/etc/colossus/database'; readOnly = $true }, @{ name = 'tmp'; mountPath = '/tmp' }) },
                @{ name = 'tls'; image = $EnvoyImage; imagePullPolicy = 'IfNotPresent'; command = @('/usr/local/bin/envoy'); args = @('-c', '/etc/anchor/config/envoy.json', '--concurrency', '1', '--log-level', 'warning')
                    securityContext = ($security + @{ runAsUser = 10002; runAsGroup = 10002 }); resources = @{ requests = @{ cpu = '25m'; memory = '32Mi' }; limits = @{ cpu = '250m'; memory = '128Mi' } }
                    ports = @(@{ name = 'https'; containerPort = 8443 }); readinessProbe = @{ tcpSocket = @{ port = 'https' }; periodSeconds = 5 }
                    volumeMounts = @(@{ name = 'config'; mountPath = '/etc/anchor/config'; readOnly = $true }, @{ name = 'https-tls'; mountPath = '/etc/anchor/tls'; readOnly = $true }, @{ name = 'tmp'; mountPath = '/tmp' }) }
            )
            volumes = @(@{ name = 'config'; configMap = @{ name = $name } }, @{ name = 'db-ca'; secret = @{ secretName = 'colossus-anchor-ca'; defaultMode = 292 } }, @{ name = 'https-tls'; secret = @{ secretName = 'colossus-anchor-https'; defaultMode = 288 } }, @{ name = 'tmp'; emptyDir = @{ sizeLimit = '16Mi' } })
        } } } }
    )
}

function Test-HelmLocalDatabaseTls {
    param($Tools, [string] $Root, [hashtable] $Manifest, [hashtable] $Database, [string] $Image)
    $namespace = $Database.namespace; $service = $Database.name
    $name = 'database-tls-' + [Guid]::NewGuid().ToString('N').Substring(0, 12)
    $script = @'
set -eu
export PGSSLMODE=verify-full PGSSLROOTCERT=/ca/ca.pem PGCONNECT_TIMEOUT=5
test "$(psql -X -q -A -t -v ON_ERROR_STOP=1 -c 'SELECT ssl FROM pg_stat_ssl WHERE pid=pg_backend_pid()')" = t
if PGSSLMODE=disable psql -X -q -A -t -c 'SELECT 1' >/dev/null 2>&1; then exit 11; fi
if PGHOST=__SHORT_HOST__ psql -X -q -A -t -c 'SELECT 1' >/dev/null 2>&1; then exit 12; fi
if PGSSLROOTCERT=/ca/wrong.pem psql -X -q -A -t -c 'SELECT 1' >/dev/null 2>&1; then exit 13; fi
printf 'verified-tls/plaintext-denied/hostname-denied/wrong-ca-denied\n'
'@
    $script = $script.Replace('__SHORT_HOST__', $service)
    $pod = @{ apiVersion = 'v1'; kind = 'Pod'; metadata = @{ name = $name; namespace = $namespace; labels = @{ 'colossus.ai/local-owner' = $Manifest.hosted.ownerId; 'app.kubernetes.io/component' = 'database-tls-probe' } }; spec = @{
        restartPolicy = 'Never'; activeDeadlineSeconds = 120; automountServiceAccountToken = $false; enableServiceLinks = $false
        securityContext = @{ runAsNonRoot = $true; runAsUser = 999; runAsGroup = 999; seccompProfile = @{ type = 'RuntimeDefault' } }
        containers = @(@{ name = 'verify'; image = $Image; imagePullPolicy = 'IfNotPresent'; command = @('/bin/sh', '-ec', $script)
            securityContext = @{ readOnlyRootFilesystem = $true; allowPrivilegeEscalation = $false; capabilities = @{ drop = @('ALL') } }
            resources = @{ requests = @{ cpu = '10m'; memory = '16Mi' }; limits = @{ cpu = '250m'; memory = '64Mi' } }
            env = @(@{ name = 'PGHOST'; value = "$service.$namespace.svc" }, @{ name = 'PGUSER'; value = $Database.user }, @{ name = 'PGDATABASE'; value = $Database.database }, @{ name = 'PGPASSWORD'; valueFrom = @{ secretKeyRef = @{ name = "$service-auth"; key = 'app-password' } } })
            volumeMounts = @(@{ name = 'ca'; mountPath = '/ca'; readOnly = $true }, @{ name = 'tmp'; mountPath = '/tmp' }) })
        volumes = @(@{ name = 'ca'; secret = @{ secretName = "$service-client-ca"; defaultMode = 292 } }, @{ name = 'tmp'; emptyDir = @{ sizeLimit = '4Mi' } })
    } }
    if (-not $Manifest.hosted.ContainsKey('probes')) { $Manifest.hosted.probes = @() }
    $receipt = @{ kind = 'Pod'; namespace = $namespace; name = $name; uid = ''; passed = $false; removed = $false }
    $Manifest.hosted.probes += $receipt
    Write-HelmLocalJson (Join-Path $Root 'manifest.json') $Manifest
    $path = Join-Path $Manifest.hosted.privateAllocation.path "$name.json"
    Write-HelmLocalPrivateFile $path (Get-HelmLocalCanonicalJson $pod)
    try {
        $created = Invoke-HelmLocalKubectl $Tools $Root $Manifest.clusterName @('create', '-f', $path, '-o', 'json') | ConvertFrom-Json -AsHashtable
        $receipt.uid = $created.metadata.uid
        Write-HelmLocalJson (Join-Path $Root 'manifest.json') $Manifest
        [void](Invoke-HelmLocalKubectl $Tools $Root $Manifest.clusterName @('-n', $namespace, 'wait', '--for=jsonpath={.status.phase}=Succeeded', "pod/$name", '--timeout=150s'))
        $log = (Invoke-HelmLocalKubectl $Tools $Root $Manifest.clusterName @('-n', $namespace, 'logs', $name)).Trim()
        if ($log -cne 'verified-tls/plaintext-denied/hostname-denied/wrong-ca-denied') { throw 'Database TLS probe did not produce its expected metadata marker.' }
        $receipt.passed = $true
    } finally {
        $raw = (Invoke-HelmLocalKubectl $Tools $Root $Manifest.clusterName @('-n', $namespace, 'get', 'pod', $name, '--ignore-not-found', '-o', 'json')).Trim()
        if ($raw) {
            $actual = $raw | ConvertFrom-Json -AsHashtable
            if ($actual.metadata.labels['colossus.ai/local-owner'] -cne $Manifest.hosted.ownerId -or ($receipt.uid -and $actual.metadata.uid -cne $receipt.uid)) { throw 'TLS probe identity changed; refusing cleanup.' }
            $receipt.uid = $actual.metadata.uid
            $deletePath = Join-Path $Manifest.hosted.privateAllocation.path "$name-delete.json"
            Write-HelmLocalPrivateFile $deletePath (Get-HelmLocalCanonicalJson @{ apiVersion = 'v1'; kind = 'DeleteOptions'; preconditions = @{ uid = $actual.metadata.uid }; propagationPolicy = 'Foreground' })
            [void](Invoke-HelmLocalKubectl $Tools $Root $Manifest.clusterName @('delete', "--raw=/api/v1/namespaces/$namespace/pods/$name", '-f', $deletePath))
            try { [void](Invoke-HelmLocalKubectl $Tools $Root $Manifest.clusterName @('-n', $namespace, 'wait', '--for=delete', "pod/$name", '--timeout=60s')) }
            catch {
                if ((Invoke-HelmLocalKubectl $Tools $Root $Manifest.clusterName @('-n', $namespace, 'get', 'pod', $name, '--ignore-not-found', '-o', 'name')).Trim()) { throw }
            }
        }
        $absent = (Invoke-HelmLocalKubectl $Tools $Root $Manifest.clusterName @('-n', $namespace, 'get', 'pod', $name, '--ignore-not-found', '-o', 'name')).Trim()
        if ($absent) { throw 'Owned TLS probe was not removed.' }
        $receipt.removed = $true
        Write-HelmLocalJson (Join-Path $Root 'manifest.json') $Manifest
    }
}

function Initialize-HelmLocalHosted {
    param($Tools, [string] $Source, [string] $Primary, [string] $Root, [string] $Cluster, [hashtable] $Manifest)
    if (-not $Manifest.hosted.ContainsKey('ownerId')) { $Manifest.hosted.ownerId = [Guid]::NewGuid().ToString('N') }
    if (-not $Manifest.hosted.ContainsKey('resources')) { $Manifest.hosted.resources = @() }
    [void](Initialize-HelmLocalPrivateAllocation -Root $Root -Manifest $Manifest)
    $material = Initialize-HelmLocalHostedMaterial $Root $Cluster $Manifest
    $Manifest.hosted.stage = 'material-retained'
    Write-HelmLocalJson (Join-Path $Root 'manifest.json') $Manifest
    $dataNamespace = "$Cluster-data"; $anchorNamespace = "$Cluster-anchors"
    $pgImage = 'docker.io/postgres:18.6@sha256:74935e72241653ca55e0414067e6d8763aceb8a810eb51b452253ec3dcfc4336'
    $envoyImage = 'docker.io/envoyproxy/envoy:distroless-v1.39.5@sha256:9d84a586119a27b50437dbb01eccd4722832741539b1ed53c8e9d6da6067a6e0'
    foreach ($namespace in @($dataNamespace, $anchorNamespace)) {
        Ensure-HelmLocalHostedObject $Tools $Root $Manifest @{ apiVersion = 'v1'; kind = 'Namespace'; metadata = @{ name = $namespace } }
    }
    $read = { param($name) Get-HelmLocalMaterialText $material $name }
    $objects = @()
    $databases = @(@{ namespace = $dataNamespace; name = 'colossus-controller-postgres'; prefix = 'controller'; user = 'colossus'; database = 'colossus'; size = '8Gi' }, @{ namespace = $anchorNamespace; name = 'colossus-anchor-postgres'; prefix = 'anchor'; user = 'anchor_app'; database = 'colossus_anchor'; size = '1Gi' })
    foreach ($db in $databases) {
        $tlsPrefix = if ($db.prefix -ceq 'controller') { 'controller' } else { 'anchorDatabase' }
        $wrongPrefix = if ($db.prefix -ceq 'controller') { 'anchorDatabase' } else { 'controller' }
        $objects += New-HelmLocalSecret $db.namespace ($db.name + '-auth') @{ 'admin-password' = & $read ($db.prefix + '-admin-password'); 'app-password' = & $read ($db.prefix + '-app-password') }
        $objects += New-HelmLocalSecret $db.namespace ($db.name + '-tls') @{ 'tls.crt' = & $read ($tlsPrefix + '-server.pem'); 'tls.key' = & $read ($tlsPrefix + '-server-key.pem') } 'kubernetes.io/tls'
        $objects += New-HelmLocalSecret $db.namespace ($db.name + '-client-ca') @{ 'ca.pem' = & $read ($tlsPrefix + '-ca.pem'); 'wrong.pem' = & $read ($wrongPrefix + '-ca.pem') }
        $objects += New-HelmLocalDatabaseObjects $db.namespace $db.name $db.database $db.user $db.size $pgImage
        $client = if ($db.prefix -ceq 'controller') {
            @{ namespaceSelector = @{ matchLabels = @{ 'kubernetes.io/metadata.name' = $Cluster } }; podSelector = @{ matchLabels = @{ 'app.kubernetes.io/component' = 'server'; 'app.kubernetes.io/instance' = 'colossus-control-plane' } } }
        } else { @{ podSelector = @{ matchLabels = @{ 'app.kubernetes.io/name' = 'colossus-chat-anchor' } } } }
        $objects += @{ apiVersion = 'networking.k8s.io/v1'; kind = 'NetworkPolicy'; metadata = @{ name = ($db.name + '-ingress'); namespace = $db.namespace }; spec = @{ podSelector = @{ matchLabels = @{ 'app.kubernetes.io/name' = $db.name } }; policyTypes = @('Ingress'); ingress = @(@{ from = @($client, @{ podSelector = @{ matchLabels = @{ 'app.kubernetes.io/component' = 'database-tls-probe' } } }); ports = @(@{ protocol = 'TCP'; port = 5432 }) }) } }
    }
    $controllerUri = "postgresql://colossus:$(& $read 'controller-app-password')@colossus-controller-postgres.$dataNamespace.svc:5432/colossus"
    $anchorUri = "postgresql://anchor_app:$(& $read 'anchor-app-password')@colossus-anchor-postgres.$anchorNamespace.svc:5432/colossus_anchor"
    $objects += New-HelmLocalSecret $Cluster 'colossus-local-database' @{ database = $controllerUri }
    $objects += New-HelmLocalSecret $Cluster 'colossus-local-database-ca' @{ 'ca.pem' = & $read 'controller-ca.pem' }
    $objects += New-HelmLocalSecret $Cluster 'colossus-local-chat' @{ 'journal-key' = & $read 'journal-key'; 'signing-key' = & $read 'signing-key'; 'wrapping-key' = & $read 'wrapping-key'; 'anchor-token' = & $read 'anchor-token' }
    $objects += New-HelmLocalSecret $Cluster 'colossus-local-anchor-ca' @{ 'ca.pem' = & $read 'anchorHttps-ca.pem' }
    $objects += New-HelmLocalSecret $anchorNamespace 'colossus-anchor-client' @{ database = $anchorUri }
    $objects += New-HelmLocalSecret $anchorNamespace 'colossus-anchor-ca' @{ 'ca.pem' = & $read 'anchorDatabase-ca.pem' }
    $objects += New-HelmLocalSecret $anchorNamespace 'colossus-anchor-state' @{ bearer = & $read 'anchor-token'; 'storage-key' = & $read 'anchor-storage-key' }
    $objects += New-HelmLocalSecret $anchorNamespace 'colossus-anchor-https' @{ 'tls.crt' = & $read 'anchorHttps-server.pem'; 'tls.key' = & $read 'anchorHttps-server-key.pem' } 'kubernetes.io/tls'
    $objects += New-HelmLocalAnchorObjects $anchorNamespace $Manifest.serverImage $envoyImage
    $objects += @{ apiVersion = 'networking.k8s.io/v1'; kind = 'NetworkPolicy'; metadata = @{ name = 'colossus-chat-anchor-ingress'; namespace = $anchorNamespace }; spec = @{ podSelector = @{ matchLabels = @{ 'app.kubernetes.io/name' = 'colossus-chat-anchor' } }; policyTypes = @('Ingress'); ingress = @(@{ from = @(@{ namespaceSelector = @{ matchLabels = @{ 'kubernetes.io/metadata.name' = $Cluster } }; podSelector = @{ matchLabels = @{ 'app.kubernetes.io/component' = 'server'; 'app.kubernetes.io/instance' = 'colossus-control-plane' } } }); ports = @(@{ protocol = 'TCP'; port = 8443 }) }) } }
    foreach ($object in $objects) { Ensure-HelmLocalHostedObject $Tools $Root $Manifest $object }
    foreach ($target in @(@($dataNamespace, 'statefulset/colossus-controller-postgres'), @($anchorNamespace, 'statefulset/colossus-anchor-postgres'), @($anchorNamespace, 'deployment/colossus-chat-anchor'))) {
        [void](Invoke-HelmLocalKubectl $Tools $Root $Cluster @('-n', $target[0], 'rollout', 'status', $target[1], '--timeout=300s'))
    }
    foreach ($namespace in @($dataNamespace, $anchorNamespace)) {
        $name = if ($namespace -ceq $dataNamespace) { 'colossus-controller-postgres-data' } else { 'colossus-anchor-postgres-data' }
        $claim = Invoke-HelmLocalKubectl $Tools $Root $Cluster @('-n', $namespace, 'get', 'pvc', $name, '-o', 'json') | ConvertFrom-Json -AsHashtable
        if ($claim.status.phase -cne 'Bound' -or -not $claim.spec.volumeName) { throw 'Hosted database PVC is not bound.' }
        $pv = Invoke-HelmLocalKubectl $Tools $Root $Cluster @('get', 'pv', $claim.spec.volumeName, '-o', 'json') | ConvertFrom-Json -AsHashtable
        if ($pv.spec.claimRef.uid -cne $claim.metadata.uid -or $pv.spec.claimRef.namespace -cne $namespace -or $pv.spec.claimRef.name -cne $name) { throw 'Hosted PV points to another claim.' }
        if (-not $Manifest.hosted.ContainsKey('volumes')) { $Manifest.hosted.volumes = @{} }
        if ($Manifest.hosted.volumes.ContainsKey($namespace) -and $Manifest.hosted.volumes[$namespace].pvUid -cne $pv.metadata.uid) { throw 'Retained Hosted database PV identity changed.' }
        $record = if ($Manifest.hosted.volumes.ContainsKey($namespace)) { $Manifest.hosted.volumes[$namespace] } else { @{ pvc = $name; pvcUid = $claim.metadata.uid; pv = $pv.metadata.name; pvUid = $pv.metadata.uid; reclaimStage = 'retain-intent' } }
        if ($record.ContainsKey('reclaimStage') -and $record.reclaimStage -ceq 'retained' -and $pv.spec.persistentVolumeReclaimPolicy -cne 'Retain') { throw 'Retained Hosted database PV reclaim policy changed.' }
        $Manifest.hosted.volumes[$namespace] = $record
        Write-HelmLocalJson (Join-Path $Root 'manifest.json') $Manifest
        if ($pv.spec.persistentVolumeReclaimPolicy -cne 'Retain') {
            if ($pv.spec.persistentVolumeReclaimPolicy -cne 'Delete') { throw 'Unexpected Hosted database PV reclaim policy.' }
            $patch = @(
                @{ op = 'test'; path = '/metadata/uid'; value = $pv.metadata.uid },
                @{ op = 'test'; path = '/metadata/resourceVersion'; value = $pv.metadata.resourceVersion },
                @{ op = 'test'; path = '/spec/claimRef/uid'; value = $claim.metadata.uid },
                @{ op = 'test'; path = '/spec/claimRef/namespace'; value = $namespace },
                @{ op = 'test'; path = '/spec/claimRef/name'; value = $name },
                @{ op = 'test'; path = '/spec/persistentVolumeReclaimPolicy'; value = 'Delete' },
                @{ op = 'replace'; path = '/spec/persistentVolumeReclaimPolicy'; value = 'Retain' }
            )
            $patchPath = Join-Path $Manifest.hosted.privateAllocation.path ('pv-retain-' + [Guid]::NewGuid().ToString('N') + '.json')
            Write-HelmLocalPrivateFile $patchPath (Get-HelmLocalCanonicalJson $patch)
            $pv = Invoke-HelmLocalKubectl $Tools $Root $Cluster @('patch', 'pv', $pv.metadata.name, '--type=json', '--patch-file', $patchPath, '-o', 'json') | ConvertFrom-Json -AsHashtable
        }
        if ($pv.metadata.uid -cne $record.pvUid -or $pv.spec.claimRef.uid -cne $record.pvcUid -or $pv.spec.persistentVolumeReclaimPolicy -cne 'Retain') { throw 'Hosted database PV retention was not established.' }
        $record.reclaimStage = 'retained'; $record.reclaimPolicy = 'Retain'
        Write-HelmLocalJson (Join-Path $Root 'manifest.json') $Manifest
    }
    foreach ($db in $databases) { Test-HelmLocalDatabaseTls $Tools $Root $Manifest $db $pgImage }
    $Manifest.hosted.stage = 'infrastructure-ready'
    $Manifest.hosted.dataNamespace = $dataNamespace; $Manifest.hosted.anchorNamespace = $anchorNamespace
    $Manifest.hosted.postgresImage = $pgImage; $Manifest.hosted.envoyImage = $envoyImage
    Write-HelmLocalJson (Join-Path $Root 'manifest.json') $Manifest
    return @{
        postgres = @{ enabled = $false }
        database = @{ external = @{ existingSecret = 'colossus-local-database'; secretKey = 'database'; tls = @{ mode = 'custom_ca'; existingSecret = 'colossus-local-database-ca'; caKey = 'ca.pem' } } }
        resources = @{ requests = @{ cpu = '500m'; memory = '512Mi' }; limits = @{ cpu = '2'; memory = '2Gi' } }
        chat = @{ enabled = $true; existingSecret = 'colossus-local-chat'; providerKey = ''; cacheSizeLimit = '1Gi'; maxActiveRuntimes = 2; idleSeconds = 300
            anchor = @{ endpoint = "https://colossus-chat-anchor.$anchorNamespace.svc/v1/anchors"; existingCaSecret = 'colossus-local-anchor-ca'; caKey = 'ca.pem' }
            projects = @{ local = New-HelmLocalControlledProject } }
    }
}
