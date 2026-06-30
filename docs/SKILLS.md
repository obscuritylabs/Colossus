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

## Skill Mode

Skill Mode is enabled by default for normal CLI and REPL agent turns. The model receives
a compact index of available skills, and full `SKILL.md` instructions are injected only
for active skills.

- Use `@skill:NAME` in a prompt for one-turn activation.
- In the REPL, use `/skill use NAME` for a sticky skill in the current process.
- Use `/skill show`, `/skill show NAME`, `/skill drop NAME`, `/skill clear`,
  `/skill new NAME`, `/skill validate PATH`, and `/skill on|off` to inspect, author,
  or manage Skill Mode.
- For one-shot CLI runs, pass repeatable `--skill NAME`.

`AgentSpec.skills` is the allowlist for an agent. If it is empty, all enabled skills are
available. Active skills must exist, must be allowed for the agent, and must not require
tools missing from the active tool catalog. Audit records store selected skill names,
versions, and sources, but not full skill instruction bodies.

## Authoring

Use the bundled `skill-creator` skill when asking the model to craft a skill:

```text
@skill:skill-creator create a skill for release checklist reviews
```

The model should use `skill.scaffold` and `skill.validate` for installed user skills.
Those tools are bounded to the user skill directory and do not execute helpers.

Create and validate skills directly from the CLI:

```bash
uv run colossus skills new release-checklist --description "Release review workflow."
uv run colossus skills validate /path/to/skills/release-checklist
```

`skills new` writes `manifest.json` and `SKILL.md`, refuses to overwrite an existing
skill unless `--force` is supplied, and defaults to the user data `skills/` directory.
Use `--path PATH` to choose a parent directory for generated files.
