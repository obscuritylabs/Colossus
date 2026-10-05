---
title: Front-end development
description: Build consistent Desktop and VS Code screens with shared UI components, design tokens, and verified preferences.
audience: developer
type: how-to
---

# Front-end development

Start with the [shared UI package](https://github.com/obscuritylabs/Colossus/tree/main/apps/ui).
It owns tokens, themes, base controls, composer behavior, and settings geometry.
Desktop and VS Code retain transport, state, capability decisions, and persistence.

## Choose the existing pattern

Before editing a screen, inspect the closest shipped surface. Providers is the reference
for compact resource inventories: modest headings, searchable tables, muted supporting
text, and ordinary settings controls. Reuse its existing presentation rather than
creating a separate visual language for another resource screen. Import shared controls
from `@colossus/ui`; host wrappers may adapt props but must not fork their presentation.

## Use design tokens first

- Use `--font-sans` and `--font-mono`, semantic `--font-size-*` values, and shared
  `--line-height-*` values. Body text, labels, help, headings, and inputs must respond
  to Compact, Comfortable, and Large text preferences. Reset input font weight when
  a parent label is emphasized.
- Use semantic surface, text, border, accent, status, and focus tokens from
  `apps/ui/styles/theme.css`. Feature CSS must not contain literal palette colors or
  alternate font families/sizes. New semantic values belong in the shared token source,
  with corresponding light, dark, and high-contrast behavior.
- Use shared radius, control-height, and settings geometry tokens where applicable.
  Fixed geometry is appropriate for icons, borders, and diagram coordinates; it is not
  a substitute for token-based typography. Dense graph labels must remain readable
  through zoom and a keyboard-accessible inspector.
- Keep focus, disabled state, contrast, dialog scrolling, and text wrapping intact.
  Do not rely on color alone to communicate state.

## Verify the rendered result

Run the affected host's check/build and browser tests. For shared UI changes, refresh
both file dependencies and validate both hosts as documented in `apps/ui/README.md`.
Use the repository's [completion gates](setup-testing.md#verification).

Inspect the actual screens in light and dark palettes, including neutral and Hacker
where supported, at compact widths and Large text. Verify that headings, forms, tables,
status labels, and dialogs keep their hierarchy and do not clip or overlap. Check
keyboard focus and accessibility for changed interactions. Include reviewed screenshots
when the work changes a user-visible flow; explain any native checks that remain blocked.
