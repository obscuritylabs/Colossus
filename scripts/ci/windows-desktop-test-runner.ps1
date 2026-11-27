[CmdletBinding()]
param(
    [Parameter(Mandatory, Position = 0)]
    [string] $TestExecutable
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false

# Cargo supplies the test executable, working directory, and runtime environment.
# Each group gets a fresh process with its original test filters.
$testGroups = @(
    @{ Name = 'Desktop library'; Arguments = @() }
    @{
        Name = 'Credential persistence'
        Arguments = @(
            'managed_runtime::credential_acceptance::native_vault_restarts_reach_managed_sidecar_mcp'
            '--ignored'
            '--exact'
        )
    }
    @{ Name = 'Setup and reset'; Arguments = @('native_catalog_', '--ignored', '--nocapture') }
    @{ Name = 'Uninstall credential cleanup'; Arguments = @('native_uninstall_', '--ignored', '--nocapture') }
)

$exitCode = 0
foreach ($group in $testGroups) {
    Write-Output "Running $($group.Name) tests"
    $testArguments = $group.Arguments
    & $TestExecutable @testArguments
    if ($exitCode -eq 0 -and $LASTEXITCODE -ne 0) { $exitCode = $LASTEXITCODE }
}
exit $exitCode
