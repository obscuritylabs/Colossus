---
name: documentation
description: Write or edit developer documentation using the locally bundled, unabridged Google Developer Documentation Style Guide. Use for READMEs, API documentation, tutorials, and how-to guides.
license: CC-BY-4.0 AND Apache-2.0
---
# Developer documentation

Use this skill when writing or editing developer documentation. Read the target
repository's documentation instructions and the user's requirements first. They take
precedence over this general style guidance. Preserve required formats, terminology,
examples, and public contracts; do not expand the requested edit to restyle other files.

Read `references/index.md` with `plugin.resource.read`, using this skill's qualified
identifier. It indexes the complete captured guide, including the word list, source
URLs, and redirects. Read the original pages relevant to the requested edit before
making style decisions. Do not substitute an inferred convention or a summary for
what the source actually recommends and its exceptions.

The following topics help locate the relevant pages in that index:

| Task | Topics |
| --- | --- |
| Heading wording and organization | Headings and titles, paragraphs, lists |
| Tone, grammar, terminology | Voice and tone, language topics, word list |
| Instructions and notices | Procedures, notes and other notices |
| API names, examples, commands | Code in text, code samples, command-line syntax, placeholders |
| Inclusive and accessible content | Inclusive language, accessibility, images |
| Attribution and snapshot maintenance | `references/sources.md`, `snapshot.json` |

The guide resources are HTML text, preserving the complete original article markup,
tables, examples, anchors, and exceptions. Large pages have ordered parts; read the
parts covering the relevant section, including any continuation or qualifications.
Platform icons have explicit text labels; apply product-specific guidance only in
its stated scope. Use the index to follow original guide links through local plugin
resources. Do not load the entire guide or fetch the website by default.

Draft for the intended reader and their next action. Check technical statements and
examples against the implementation and the repository's validation requirements.
Apply relevant style guidance without changing technical meaning.

These files are read-only reference material. Selecting or reading this skill grants
no execution, filesystem-write, or network authority. Use the existing policy-mediated
plugin readers; do not run scripts, modify the reference installation, or request new
permissions just to read guidance. An authorized documentation edit still uses the
normal workspace write tools and permissions.

This is an unabridged snapshot of the published guide's article text, not a summary
or an official Google distribution. Site navigation and generated page summaries
are excluded. External image files are not redistributed; their original URLs and
alternative text remain in the articles. No guidance is paraphrased or omitted.
Source: [Google Developer Documentation Style Guide](https://developers.google.com/style),
by Google. Prose is licensed under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/);
code samples under [Apache 2.0](https://www.apache.org/licenses/LICENSE-2.0).
See `references/sources.md`, `LICENSE.txt`, and `LICENSE-APACHE.txt`.
