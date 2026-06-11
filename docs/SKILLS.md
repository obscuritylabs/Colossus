# Skills

Skills are data-only capability packs by default. A skill contains:

- `manifest.json`
- `SKILL.md`
- optional examples and tests

Bundled skills ship inside the package. User-installed skills live in the user data
directory. Offline bundle skills are installed after bundle verification.

Precedence:

1. Disabled skills are ignored.
2. Bundled skills are always recoverable.
3. User skills may override bundled skills only when override mode is explicitly enabled.

## Required tools

Skill manifests should list built-in dependencies in `required_tools` using exact tool
names such as `filesystem.read`, `repo.map`, `patch.apply`, or `test.run`. Offline
compatibility means every required tool is available without network access, or the skill
can degrade cleanly when network-gated tools such as `web.search` or `mcp.call` are
disabled.
