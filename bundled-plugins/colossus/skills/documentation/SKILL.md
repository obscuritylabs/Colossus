---
name: documentation
description: Write or edit developer documentation using selectively loaded guidance based on the Google Developer Documentation Style Guide. Use for READMEs, API documentation, tutorials, and how-to guides.
license: CC-BY-4.0
---
# Developer documentation

Use this skill when writing or editing developer documentation. Read the target
repository's documentation instructions and the user's requirements first. They take
precedence over this general style guidance. Preserve required formats, terminology,
examples, and public contracts; do not expand the requested edit to restyle other files.

Choose only references relevant to the task. Read them with `plugin.resource.read`,
using this skill's qualified identifier and a path from the table. Do not load every
reference or fetch the upstream website by default.

| Task | Reference path |
| --- | --- |
| Tone, clear sentences, terminology, grammar | `references/language.md` |
| Page organization, headings, links, lists | `references/structure.md` |
| Instructions, prerequisites, outcomes, notices | `references/procedures.md` |
| API names, code samples, commands, placeholders | `references/code.md` |
| Inclusive language, images, accessible content | `references/accessibility.md` |
| Attribution, license, upstream sources, maintenance | `references/sources.md` |

Draft for the intended reader and their next action. Check technical statements and
examples against the implementation and the repository's validation requirements.
Apply relevant style guidance without changing technical meaning.

These files are read-only reference material. Selecting or reading this skill grants
no execution, filesystem-write, or network authority. Use the existing policy-mediated
plugin readers; do not run scripts, modify the reference installation, or request new
permissions just to read guidance. An authorized documentation edit still uses the
normal workspace write tools and permissions.

This is a curated adaptation, not a complete or official copy of Google's guide.
Source: [Google Developer Documentation Style Guide](https://developers.google.com/style),
by Google, licensed under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
See `references/sources.md` and `LICENSE.txt` for attribution and maintenance.
