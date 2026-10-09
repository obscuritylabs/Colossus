---
title: Author a terminal theme
description: Create, load, and refine a custom Colossus terminal theme.
audience: user
type: how-to
icon: lucide/paintbrush
---

# Author a terminal theme

A custom theme is a small TOML or JSON file. It starts from one of the
[built-in themes](../use/theme.md#built-in-themes), then changes only the parts you name.
Colossus loads theme files when the terminal starts.

## 1. Find a theme folder

In the terminal UI, enter:

```text
/theme list
```

The **Custom theme search locations** section shows the folders Colossus reads. If
your active configuration is `.colossus/config.yaml` in a repository, use
`.colossus/themes/` in that repository. Create the folder if it does not exist. For
another configuration location, use its adjacent `themes/` folder or the user theme
folder shown by `/theme list`.

## 2. Save a theme file

Create `midnight.toml` in that folder with this content:

```toml
schemaVersion = 1
name = "midnight"
base = "default"
title = "Colossus Midnight"
caret = "›"
continuation = "…"
spinner = "arc"

[prompt]
left = "#9BB7FF"
indicator = "#9BB7FF"

[styles.assistant]
foreground = "#E6E9FF"

[styles.tool]
foreground = "#79C0FF"
bold = true

[styles.warning]
foreground = "#FFD479"
bold = true
```

`schemaVersion` must be `1`. Use a unique theme `name` made of letters, digits, and
underscores. Colors use `#RRGGBB`. Unspecified colors and text styles inherit the
`base` theme.

You can also set `[prompt]` colors for `right` and `continuation`; add
`[styles.activity]`, `[styles.thinking]`, `[styles.success]`,
`[styles.error]`, or `[styles.meta]`. Each style accepts `foreground`, `bold`,
`dim`, and `italic`. Available spinners are `dots`, `line`, `arc`,
`bouncingBar`, and `aesthetic`.

## 3. Reload and select it

Exit and restart the terminal UI so Colossus reads the new file. Then enter:

```text
/theme list
/theme validate
/theme preview midnight
/theme midnight
```

The list should include `midnight`, validation should report the loaded library as
valid, and the preview should show its colors without applying them. The final command
saves it as your active theme. You can also select it in the `/theme` browser.

When you edit an already selected custom theme, restart Colossus and select the theme
again. The active preference keeps the previously selected version until you reselect
it. If Colossus reports an invalid theme while starting, check the TOML syntax, theme
name, color values, and folder shown by `/theme list`.
