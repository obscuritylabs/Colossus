---
title: Skills, packs, and workflow configuration
description: Exact roots and controls for Colossus workflows, skills, and packs.
audience: operator
type: reference
---

# Skills, packs, and workflow configuration

These fields select workspace-relative extension roots and control skill loading.

```yaml
workflows:
  repository: .colossus/workflows
  user: workflows
skills:
  enabled: true
  allowUserOverrides: false
  bundled: bundled-skills
  repository: .colossus/skills
  user: skills
  disabled: []
packs:
  installRoot: .colossus/packs
```

## Fields

| Field | Values / constraint |
| --- | --- |
| `workflows.repository` | Repository-owned workflow root |
| `workflows.user` | User workflow root |
| `skills.enabled` | Enables or disables skill discovery |
| `skills.allowUserOverrides` | Allows user skills to replace lower-precedence names |
| `skills.bundled` | Bundled skill root |
| `skills.repository` | Repository-owned skill root |
| `skills.user` | User skill root |
| `skills.disabled` | Unique skill directory names to suppress |
| `packs.installRoot` | Pack installation root |

Relative paths resolve from the selected workspace. Each `skills.disabled` entry is a
unique 1–128 character directory name made from ASCII letters, digits, `.`, `_`, and
`-`. Pack installation uses only `packs.installRoot`.

See [Workflow authoring](../../extend/workflows/authoring.md),
[Skills](../../extend/skills.md), and [Packs](../../extend/packs.md) for operational
guidance.

Return to the [configuration overview](../configuration.md).
