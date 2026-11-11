# Sources, licensing, and snapshot maintenance

The bundled reference reproduces the complete article text of the English
[Google Developer Documentation Style Guide](https://developers.google.com/style),
by Google. It is not a condensed adaptation or an official Google distribution.
`snapshot.json` records the capture time, all source URLs, upstream revision dates,
original response SHA-256 hashes, navigation, redirects, resource parts, and media
references. The manifest's source paths are relative to
`scripts/fixtures/google-style-guide/` in the source checkout. That directory retains
the compressed original HTML responses for offline verification. Installations include
the extracted articles and their provenance manifest.

The initial full snapshot contains all 70 pages in Google's navigation and follows
in-guide links to their canonical destinations. One malformed upstream link,
`/style/style/word-list`, returned HTTP 404 and is recorded rather than silently
treated as captured content. External sites linked by the guide are not part of
this snapshot.

## Reproduction and mechanical changes

The agent-readable resources retain original article HTML rather than relying on
Google's Markdown export, which drops some platform annotations. The extractor
removes site chrome and Google's generated Page Summary widget, retains the page
title, and inserts explicit scope labels beside Android, Google Cloud, and Google
Workspace icons. It adds source attribution and a base URL. No article wording,
code samples, tables, examples, exceptions, or original anchors are removed or
rewritten. Large articles are split at line boundaries without losing any bytes;
concatenating their parts reconstructs the complete annotated article.

The HTML is inert text for the plugin resource reader. It requires neither a
browser nor a network connection. Original in-guide links map to local resources
through `references/index.md`; original fragments remain in the article markup.
External links and image URLs are retained as source references, not runtime fetch
instructions. Three unique external image files are not bundled. Their original
alternative text is preserved; the media list in `snapshot.json` identifies each
occurrence. Google's site policy does not generally license media under the prose
license. The Android/Cloud/Workspace marks are represented by text annotations;
their logo assets are not redistributed.

## Attribution and licenses

This reference reproduces and mechanically modifies work created and
[shared by Google](https://developers.google.com/terms/site-policies), under the
[Creative Commons Attribution 4.0 International License](https://creativecommons.org/licenses/by/4.0/).
Every article identifies its original source. `LICENSE.txt` contains CC BY 4.0.
Original code samples are retained under the
[Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0), included in
`LICENSE-APACHE.txt`. Original copyright, attribution, and license notices are
retained in the source snapshot. These licenses apply to the referenced Google
content, not to the surrounding Colossus implementation. Do not imply Google
endorsement or apply the prose license to trademarks, externally linked material,
or separately licensed media.

## Refresh the snapshot

The maintainer-only script `scripts/google_style_guide.py` belongs to the Colossus
source checkout, not this runtime skill. It uses Python's standard library.

1. Run `python3 scripts/google_style_guide.py --fetch /absolute/new/staging-directory`.
   This explicitly downloads Google's English pages, follows in-guide links, checks
   their license notices, and fails if extraction or validation is incomplete.
2. Review the source/page inventory diff, licensing and media changes, broken links,
   and generated article changes. Verify that upstream examples and scope annotations
   remain intact. New navigation pages or separately licensed content require review.
3. Copy the staged `upstream/` to `scripts/fixtures/google-style-guide/upstream/`.
   Replace the skill's `references/guide/`, generated `references/index.md`, and
   `snapshot.json` with the corresponding staging files. Remove superseded files
   from both destinations so they match the staged snapshot.
4. Run `python3 scripts/google_style_guide.py --check` and
   `python3 -B -m unittest discover -s scripts/tests -p 'test_google_style_guide.py'`.
   Run the bundled-plugin/runtime policy tests and the repository completion gates.

Normal builds and CI verify the checked-in snapshot offline; they do not refresh it.
Installed snapshots follow Colossus releases and the existing plugin lifecycle.
Active runs retain their immutable captured resources. Reading the skill never
updates an installation or grants execution, filesystem-write, or network authority.
