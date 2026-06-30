# Skills

Skills are agent instructions plus optional read-only resources. They are not executable
plugins. Use a standalone skill for pure guidance or reference material; use a pack when
the capability needs scripts, binaries, Docker assets, MCP servers, integrations, or trust
metadata.

Supported skill layouts:

- Agent Skills-compatible: `SKILL.md` with YAML frontmatter containing `name` and
  `description`.
- `.agents Protocol` compatibility: `skill.md` is accepted only when `SKILL.md` is
  absent. New skills use `SKILL.md`; a directory with both names is invalid.
- Colossus manifest-backed: `manifest.json` plus `SKILL.md`.
- Hybrid: both files. When both exist, the frontmatter `name` and `description` must
  match `manifest.json`.
- Optional resource directories: `references/`, `scripts/`, `assets/`, `examples/`, and
  `tests/`.

Bundled skills ship inside the package. Workspace skills live under `.agents/skills`.
User-global skills live under `~/.agents/skills`. Legacy Colossus user skills live in
the user data directory. Pack-backed skills live under `skills/` inside bundled or
installed packs. Offline bundle skills are installed after bundle verification.

Precedence:

1. Disabled skills are ignored.
2. Bundled system skills are always recoverable.
3. Bundled first-party pack skills are next.
4. Installed pack skills are next.
5. Legacy user skills are next.
6. User-global `~/.agents/skills` skills are next.
7. Workspace `.agents/skills` roots are last, from repository root down to the active
   workspace directory. Later skills may override earlier skills only when override mode is
   explicitly enabled.

`skills list` and REPL skill status report duplicate names so shadowed skills are visible.

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

## Resources

Skill resources are available only through explicit read-only tools:

- `skill.resource.list` lists files under allowed resource directories for an active
  skill.
- `skill.resource.read` reads one bounded text-safe resource file from an active skill.

Resource access is restricted to active skill roots, safe relative paths, allowed resource
directories, and bounded file sizes. Resource reads are audited by skill name, path, and
size; resource contents are not injected automatically.

Code files may appear in `scripts/` as resources for inspection or copying into an
approved workflow. Colossus does not execute scripts directly from skills. Executable
capability must be declared by a pack.

## Authoring

Use the bundled `skill-creator` skill when asking the model to craft a skill:

```text
@skill:skill-creator create a skill for release checklist reviews
```

The model should use `skill.scaffold` for new installed user skills, then
`skill.validate`. For existing user skills, it should use `skill.inspect` and
`skill.read` first, then `skill.write` for targeted edits to `SKILL.md`,
`manifest.json`, or files under allowed resource directories. Overwriting an existing
file requires the `expected_sha256` returned by `skill.read` or `skill.inspect`, which
prevents stale edits. `skill.scaffold` and `skill.write` do not execute helpers.

For normal repo-local authoring, create and edit skills under `.agents/skills/NAME`
with the regular workspace filesystem and command tools. Local skills are code-like
workspace artifacts and can include validation fixtures or helper resources. Keep
`.colossus/` for Colossus runtime/control state; generic workspace tools intentionally
cannot write there.

Create and validate skills directly from the CLI:

```bash
uv run colossus skills new release-checklist --description "Release review workflow."
uv run colossus skills new release-checklist --agent-compatible --resources references,tests
uv run colossus skills new release-checklist --pack ./my-pack
uv run colossus skills install .agents/skills/release-checklist
uv run colossus skills validate /path/to/skills/release-checklist
```

`skills new` writes `manifest.json` and `SKILL.md`, refuses to overwrite an existing
skill unless `--force` is supplied, and defaults to `.agents/skills` in the current
workspace. Use `--user` for the legacy user data `skills/` directory, `--path PATH` to
choose a parent directory for generated files, or `--pack PATH` to create under
`PATH/skills`.

`skills install PATH` validates a local skill directory and copies it into
`~/.agents/skills/NAME`. It refuses to overwrite an existing global skill unless
`--force` is supplied.

`skills validate` accepts frontmatter-only skills and manifest-backed skills. It checks
frontmatter, manifest consistency, path safety, allowed resource directories, text-safe
resource files, and oversized content.
