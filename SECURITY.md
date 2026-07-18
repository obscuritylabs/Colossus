# Security Policy

## Supported Versions

Security fixes are applied to the active development line unless a release branch is
explicitly documented in `CHANGELOG.md`.

## Reporting a Vulnerability

Do not open public issues for suspected vulnerabilities. Send a private report to the
project maintainers with:

- Affected version or commit.
- Environment and installation method.
- Reproduction steps or proof of concept.
- Expected impact.
- Any logs or audit excerpts that are safe to share.

The maintainers should acknowledge receipt within 5 business days, provide an initial
triage result when practical, and coordinate disclosure timing before public details are
published.

## Security Scope

Security-sensitive areas include:

- Tool and subprocess execution.
- Approval and policy decisions.
- Audit log integrity and redaction.
- Bundle verification and offline installation.
- Skill loading and override behavior.
- Model provider configuration and credential handling.

See the
[security architecture](docs/develop/security-architecture.md) for the implementation
security model and the
[operator security guides](docs/admin/access-and-approvals.md) for deployment controls.
