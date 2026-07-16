# Troubleshooting

Start with the same bounded diagnostics for every Rust deployment:

```bash
colossus --config .colossus/config.yaml config show
colossus --config .colossus/config.yaml state doctor
colossus --config .colossus/config.yaml policy doctor
colossus --config .colossus/config.yaml sandbox doctor
colossus --config .colossus/config.yaml projection status
colossus --config .colossus/config.yaml provider profiles
colossus --config .colossus/config.yaml models routes
colossus --config .colossus/config.yaml tools list
```

These commands retain credential references and bounded metadata; they do not print
secret values.

## Config Does Not Parse

Rust YAML denies unknown fields. Compare exact camelCase/snake_case names with
[Configuration](CONFIGURATION.md), then run `config show`. Common failures are:

- a network provider without `baseUrl`;
- an OpenAI Responses profile without `credentialReference`;
- a remote HTTP origin absent from `sandbox.networkDestinations`;
- relative sandbox filesystem/executable paths;
- Git tools without exactly one executable named `git` or `git.exe`;
- `shell.run` without an exact executable;
- OCI images without immutable SHA-256 digests;
- Windows `windows_job` network effects when the host cannot create a temporary
  AppContainer loopback exemption or package-scoped dynamic WFP filters. Colossus fails
  closed before launch; run `sandbox doctor` and use a host identity permitted to manage
  the Windows filtering engine.

## Echo Works But The Model Does Not

```bash
colossus --config .colossus/config.yaml echo ok
colossus --config .colossus/config.yaml provider doctor PROFILE
colossus --config .colossus/config.yaml provider models PROFILE
colossus --config .colossus/config.yaml run \
  "Reply with exactly: connected"
```

Check, in order: role-to-profile route, credential environment variable, provider policy
action, exact network origin, TLS trust, model identifier, and response shape. HTTP 200
without released assistant content is not a successful turn. Incomplete streams become
unknown rather than synthesized completion.

## Policy Denied An Effect

Approval mode cannot repair a deny. Confirm the exact action in `policy.allow_actions`
or `approval_actions`, then confirm the matching filesystem, executable, environment, or
network obligation in `sandbox`.

For an approval obligation, place the global flag before the subcommand:

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  run "Apply the approved change"
```

OPA transport failures, invalid/missing decision fields, unhealthy bundles, oversized
input, or unverifiable decision-log masking fail closed. Use `policy doctor`.

## Tool Is Missing

`agent.tools` is an exact allowlist. Integration operations appear only after a connected
canonical lifecycle; MCP operations require configured servers and tool allowlists;
goal tools appear only during active goal lineage. Run `tools list` to see the actual
catalog and effect identities.

## Workspace Or Filesystem Looks Wrong

The process working directory is the workspace. Restart Colossus from the intended
repository and pass an absolute config path. Then ensure YAML contains a matching
absolute root. Changing cwd does not add authorization.

Symlinks, traversal, `.colossus` control-state writes, undeclared roots, oversized reads,
and post-effect-denied content are intentionally unavailable.

## Worker Problems

```bash
colossus --config .colossus/config.yaml worker --status
colossus --config .colossus/config.yaml worker --shutdown
```

Only one redb writer lease is allowed. A healthy worker owns it; CLI/TUI use authenticated
IPC. Wrong-key, stale, replayed, malformed, or incorrectly permissioned endpoints fail
without embedded fallback. If no worker endpoint exists, the same runtime embeds safely.

## Recovery Mode Or Audit Failure

```bash
colossus --config .colossus/config.yaml audit verify
colossus --config .colossus/config.yaml audit anchor-status
colossus --config .colossus/config.yaml audit show --limit 50
```

Chain, anchor, checkpoint, decryption, or projection-position failure activates read-only
recovery and blocks new effects. Preserve the state file, key identity, secure anchor,
and diagnostic output. Do not delete or rewrite records to make verification pass.

An `outcome_unknown` event means execution may have escaped before terminal evidence.
Investigate externally and use only the operation-specific explicit recovery path; never
blindly rerun a non-idempotent effect.

## Memory Search Is Degraded

```bash
colossus --config .colossus/config.yaml memories index status
colossus --config .colossus/config.yaml memories index sync
colossus --config .colossus/config.yaml memories index rebuild
```

Canonical memories remain readable even when Tantivy or Chroma is unavailable. Rebuild
is an explicit destructive replacement of only the disposable projection.

## Report Safely

Include the exact command, bounded doctor output, run ID, effect action, policy decision
ID/revision, and audit sequence. Never include API keys, tokens, authentication headers,
private keys, key material, decrypted payloads, or unredacted quarantined content.
