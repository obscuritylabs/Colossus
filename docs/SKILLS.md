# Skills

Skills are declarative agent instructions with optional bounded resources. They are not
executable plugins. Any capability that needs a process, binary, container, MCP server,
or integration belongs in a verified [pack](PACKS.md).

## Layout And Discovery

A skill directory contains `SKILL.md` with YAML frontmatter (`name` and `description`).
An optional strict `manifest.json` may declare triggers, required tools, permissions,
offline compatibility, and resources. When both files contain identity metadata they
must agree.

Allowed resource directories are `references/`, `scripts/`, `assets/`, `examples/`, and
`tests/`. Files under `scripts/` remain data for inspection or copying; skill activation
never executes them.

Roots are explicit YAML:

```yaml
skills:
  enabled: true
  allowUserOverrides: false
  bundled: bundled-skills
  repository: .colossus/skills
  user: skills
  disabled: []
```

Later roots win duplicate names only when `allowUserOverrides` is true. Duplicates remain
visible instead of silently disappearing.

## Use

```bash
colossus --config .colossus/config.yaml skills list
colossus --config .colossus/config.yaml skills duplicates
colossus --config .colossus/config.yaml skills show coding
colossus --config .colossus/config.yaml skills compose \
  "Implement this" --skill coding
colossus --config .colossus/config.yaml run --skill coding \
  "Implement the approved plan"
```

In the TUI, `/skills`, `/skill use NAME`, `/skill active`, `/skill show NAME`,
`/skill clear`, `/skill resources NAME`, and `/skill read NAME PATH` operate on the same
runtime service. Full instructions are injected only for active skills. Required tools
must exist in the active catalog before composition succeeds.

## Resources

```bash
colossus --config .colossus/config.yaml skills resources coding
colossus --config .colossus/config.yaml skills read coding references/checklist.md
```

Resource reads accept safe relative paths under allowed directories, reject symlinks and
non-text/oversized content, cross the effect gateway, and record metadata rather than
file bodies in audit evidence.

## Authoring

Create an installed user skill through the guarded service:

```bash
colossus --config .colossus/config.yaml --approval-mode ask \
  skills scaffold release-checklist "Review native release readiness" \
  --instructions "Verify gates, artifacts, checksums, and audit evidence." \
  --resource-dir references --resource-dir tests
colossus --config .colossus/config.yaml skills inspect release-checklist
colossus --config .colossus/config.yaml skills validate release-checklist
```

For a targeted edit, read first and use the returned hash:

```bash
colossus --config .colossus/config.yaml skills file-read \
  release-checklist SKILL.md
colossus --config .colossus/config.yaml --approval-mode ask skills write \
  release-checklist SKILL.md 'Updated instructions' --expected-sha256 SHA256
```

`skills write` accepts literal content. For large content, prefer model-visible
`skill.write` or an application API caller that passes the complete string without shell
ambiguity. Optimistic hashes prevent stale overwrite.

Validate and install a workspace-local directory:

```bash
colossus --config .colossus/config.yaml skills validate \
  .colossus/skills/release-checklist --local
colossus --config .colossus/config.yaml --approval-mode ask \
  skills install .colossus/skills/release-checklist
```

Scaffold, write, and install are independently authorized and audited. Generic file tools
cannot use `.colossus` control state as an arbitrary mutation target.
