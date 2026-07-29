param(
    [Parameter(Mandatory = $true)]
    [string]$Target
)

$ErrorActionPreference = "Stop"
$PSNativeCommandUseErrorActionPreference = $true
$binary = Join-Path $env:GITHUB_WORKSPACE "target/$Target/release/colossus.exe"
$metadata = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
$version = ($metadata.packages | Where-Object name -eq "colossus-cli").version
$package = "colossus-$version-$Target"

$smoke = Join-Path $env:RUNNER_TEMP "colossus-release-smoke-$Target"
Remove-Item -Recurse -Force $smoke -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force (Join-Path $smoke "workflows") | Out-Null
Copy-Item release/smoke-config.yaml (Join-Path $smoke "config.yaml")
Push-Location $smoke
try {
    $versionOutput = (& $binary --version | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or -not $versionOutput.StartsWith("colossus ")) { throw "version command failed" }
    & $binary --config config.yaml config show | Out-Null
    & $binary --config config.yaml run connected | Set-Content -Encoding utf8 result.json
    $result = Get-Content -Raw result.json | ConvertFrom-Json
    if ($result.output -ne "connected" -or $result.profile -ne "echo" -or $result.event_count -lt 3) { throw "offline smoke failed" }
    & $binary --config config.yaml audit verify | Set-Content -Encoding utf8 audit.json
    $audit = Get-Content -Raw audit.json | ConvertFrom-Json
    if ($audit.last_sequence -lt 1 -or $audit.checkpoint.global_sequence -ne $audit.last_sequence) { throw "audit smoke failed" }
} finally {
    Pop-Location
}

$stage = Join-Path $env:RUNNER_TEMP $package
$dist = Join-Path $PWD "dist"
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $stage, $dist | Out-Null
Copy-Item $binary (Join-Path $stage "colossus.exe")
Copy-Item release/install.ps1 (Join-Path $stage "install.ps1")
Copy-Item LICENSE (Join-Path $stage "LICENSE")
Copy-Item README.md (Join-Path $stage "README.md")
$archive = Join-Path $dist "$package.zip"
Compress-Archive -Path $stage -DestinationPath $archive -Force
$hash = (Get-FileHash -Algorithm SHA256 $archive).Hash.ToLowerInvariant()
"$hash  $package.zip" | Set-Content -Encoding ascii "${archive}.sha256"

$extract = Join-Path $env:RUNNER_TEMP "colossus-install-extract-$Target"
$prefix = Join-Path $env:RUNNER_TEMP "colossus-install-prefix-$Target"
$installedSmoke = Join-Path $env:RUNNER_TEMP "colossus-install-smoke-$Target"
Remove-Item -Recurse -Force $extract, $prefix, $installedSmoke -ErrorAction SilentlyContinue
Expand-Archive -LiteralPath $archive -DestinationPath $extract
& (Join-Path $extract "$package/install.ps1") -Prefix $prefix
New-Item -ItemType Directory -Force (Join-Path $installedSmoke "workflows") | Out-Null
Copy-Item release/smoke-config.yaml (Join-Path $installedSmoke "config.yaml")
$installed = Join-Path $prefix "bin/colossus.exe"
Push-Location $installedSmoke
try {
    & $installed --config config.yaml run installed-offline | Set-Content -Encoding utf8 result.json
    $result = Get-Content -Raw result.json | ConvertFrom-Json
    if ($result.output -ne "installed-offline" -or $result.profile -ne "echo") { throw "installed smoke failed" }
    & $installed --config config.yaml audit verify | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "installed audit failed" }
} finally {
    Pop-Location
}

$bundleRoot = Join-Path $env:RUNNER_TEMP "colossus-bundle-smoke-$Target"
$bundleStage = Join-Path $bundleRoot "stage"
$bundle = Join-Path $bundleRoot "bundle"
$bundlePrefix = Join-Path $bundleRoot "prefix"
$workflows = Join-Path $bundleRoot "workflows"
Remove-Item -Recurse -Force $bundleRoot -ErrorAction SilentlyContinue
$artifactDirectory = Join-Path $bundleStage "artifacts/$Target"
New-Item -ItemType Directory -Force $artifactDirectory, $workflows | Out-Null
Copy-Item $binary (Join-Path $artifactDirectory "colossus.exe")
Copy-Item LICENSE (Join-Path $bundleStage "LICENSE")
$config = [ordered]@{
    schemaVersion = 2
    access = [ordered]@{
        profile = "pinned"
        tools = [ordered]@{ include = @("echo"); exclude = @() }
        actions = [ordered]@{
            allow = @("bundle.verify")
            requireApproval = @("bundle.key.inspect", "pack.trust.add", "bundle.build", "bundle.install")
            deny = @()
        }
    }
    storage = [ordered]@{
        path = (Join-Path $bundleRoot "state.redb")
        keys = [ordered]@{
            kind = "environment"
            journal_variable = "COLOSSUS_BUNDLE_JOURNAL_KEY"
            journal_key_id = "release-bundle-journal-v1"
            signing_variable = "COLOSSUS_BUNDLE_CHECKPOINT_KEY"
            anchor_path = (Join-Path $bundleRoot "anchor.json")
        }
    }
    policy = [ordered]@{ kind = "built_in"; require_post_effect = $false }
    workflows = [ordered]@{ repository = $workflows; user = $workflows }
    providers = [ordered]@{
        profiles = [ordered]@{
            echo = [ordered]@{ kind = "echo"; baseUrl = $null; credentialReference = $null; timeoutMs = 5000 }
        }
    }
    models = [ordered]@{
        profiles = [ordered]@{
            echo = [ordered]@{
                providerProfile = "echo"
                model = "echo"
                contextWindowTokens = 32768
                maxOutputTokens = 4096
                capabilities = [ordered]@{ toolCalls = $true; streaming = $true }
            }
        }
        roles = [ordered]@{ primary = "echo" }
    }
    agent = [ordered]@{ maxTurns = 2 }
    subagents = [ordered]@{ maxConcurrent = 1 }
    sandbox = [ordered]@{
        backend = "native"; profile = "release-bundle-smoke-v1"; allowBrokerFallback = $false
        helperPath = $null; ociRuntime = $null; ociImage = $null; ociProxyImage = $null
        filesystem = @([ordered]@{ root = $bundleRoot; mode = "write" })
        executables = @(); environment = @(); networkDestinations = @()
        timeoutMs = 30000; maxOutputBytes = 1048576; maxProcesses = 1
        maxMemoryBytes = 67108864; maxConcurrency = 1
    }
}
$configPath = Join-Path $bundleRoot "config.yaml"
$config | ConvertTo-Json -Depth 12 | Set-Content -Encoding utf8 $configPath
$keyInfo = & $binary --config $configPath --approval-mode full-access bundle key-info `
    --signing-key-reference env:COLOSSUS_BUNDLE_SIGNING_SEED | ConvertFrom-Json
& $binary --config $configPath --approval-mode full-access packs trust add colossus `
    --public-key $keyInfo.public_key | Out-Null
$build = & $binary --config $configPath --approval-mode full-access bundle build `
    $bundleStage $bundle --name colossus-offline --version $version --publisher colossus `
    --created-at 2026-07-11T00:00:00Z --source-revision $env:GITHUB_SHA `
    --signing-key-reference env:COLOSSUS_BUNDLE_SIGNING_SEED | ConvertFrom-Json
& $binary --config $configPath bundle verify $bundle | Out-Null
$install = & $binary --config $configPath --approval-mode full-access bundle install `
    $bundle --prefix $bundlePrefix | ConvertFrom-Json
$bundleInstalled = Join-Path $bundlePrefix "bin/colossus.exe"
$bundleResult = & $bundleInstalled --config $configPath run bundle-installed | ConvertFrom-Json
if ($build.targets.Count -ne 1 -or $build.targets[0] -ne $Target) { throw "unexpected bundle targets" }
if ($install.target -ne $Target -or $bundleResult.output -ne "bundle-installed") { throw "unexpected bundle install result" }
& $bundleInstalled --config $configPath audit verify | Out-Null
if ($LASTEXITCODE -ne 0) { throw "bundle-installed audit verify failed" }
