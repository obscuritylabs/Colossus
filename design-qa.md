# Desktop activity timeline design QA

## Sources and implementation

- Run Capsule reference: `/Users/alex/.codex/generated_images/01a001d9-b94f-7633-b625-6849323278a2/exec-f4cb4d03-c851-4e7c-9314-856424de91da.png`
- Working Thread reference: `/Users/alex/.codex/generated_images/01a001d9-b94f-7633-b625-6849323278a2/exec-577a4703-92de-4bd2-b5c1-6b62e0426e4c.png`
- Native Run Capsule capture: `/private/tmp/colossus-run-capsule.png`
- Native Working Thread capture: `/private/tmp/colossus-working-thread-final.png`
- Implementation: `apps/desktop/src/components/RunTimeline.tsx`
- Styling: `apps/desktop/src/styles.css`
- Deterministic comparison state: `apps/desktop/src/dev/operations-studio-fixture.ts`

## Viewport and state

- Native Tauri window capture: 1229 × 768 px.
- Both implementations used the same completed run, five canonical tool calls, three released `reasoning_summary` notes, one denied tool action, and one final response.
- The temporary Capsule / Working thread switch was enabled only by the `activity-comparison` development fixture.

## Comparison

### Run Capsule

- Matches the reference's bounded card, compact status header, exception count, and row-level disclosures.
- Integrates cleanly with the current Colossus sidebar, agent strip, thread width, and composer dock.
- Remains the strongest dense inspection view, but the surrounding card visually flattens the semantic difference between released model thinking and tool execution.

### Working Thread

- Matches the reference's temporal spine and interleaves released model notes with tool calls in canonical sequence.
- Leads tool rows with their released human-readable summary and retains the raw tool name, state, timestamp, lifecycle, input, and preview as secondary or disclosed information.
- Makes the policy-denied action and subsequent read-only pivot legible as one narrative without fabricating hidden reasoning.
- Uses the whole-run disclosure to keep long completed turns compact while remaining open during active, waiting, or exceptional work.

## Interaction checks

- Switched between Capsule and Working Thread without changing run data.
- Expanded a denied `shell.run` item and verified the released raw command and lifecycle were present.
- Collapsed and re-expanded the whole Working Thread through its accessible disclosure action.
- Verified the comparison selector exposes pressed state and remains absent outside its development fixture.

## Findings and resolution

- Resolved: raw tool names no longer dominate the visual hierarchy; released action summaries lead.
- Resolved: actual released thinking is shown between tool calls; no synthetic thinking or hidden chain-of-thought is created.
- Resolved: repeated lifecycle transitions remain coalesced by call ID and raw details remain inspectable.
- Resolved: the activity header and body adapt at compact widths without requiring a second horizontal surface.
- Accepted difference: the references isolate the conversation, while the native captures include the production Space sidebar, agent strip, and composer dock.

## Downselection

The **Working Thread** is the selected production default. It communicates how Colossus works more honestly because released thinking, actions, failures, and pivots remain chronological. The Run Capsule remains implemented as a development comparison variant and a possible future dense inspection mode.

## Timeline rail alignment and status polish — 2026-08-15

### Source and rendered evidence

- Source visual truth: `/var/folders/v5/10wplgc941b7wgrx5yvd64540000gp/T/codex-clipboard-950aae64-9f9f-4812-9e07-0d10b2669223.png` (138 × 450 px).
- Browser-rendered full implementation: `/Users/alex/tools/Colossus/design-qa-timeline-full.jpg` (1280 × 720 px).
- Focused implementation crop: `/Users/alex/tools/Colossus/design-qa-timeline-implementation.jpg` (826 × 361 px).
- CSS viewport: 1280 × 720 at device pixel ratio 2. A second responsive measurement used a 760 × 720 CSS viewport and was reset after verification.
- State: completed Working Thread with interleaved reasoning notes, completed tools, and one failed action. The active-state fixture also covered completed and approval-waiting tools.

### Full-view comparison evidence

- The temporal spine remains visually subordinate to the conversation and does not change the thread's density, typography, copy, disclosure controls, or composer placement.
- The completed run mark and completed tool nodes use the existing semantic green with a restrained halo. Requested/started work maps to the existing cyan token, approval-waiting remains amber, and failures remain red.
- Browser console errors checked: none.

### Focused comparison evidence

- The summary mark center measured at 444 px, the rail center at 444.5 px, and completed node centers at 444.5 px in the 1280 px viewport.
- At the 760 px responsive breakpoint, the summary mark measured at 54 px and both rail and node centers at 54.5 px.
- The focused crop shows completed nodes clearly without making the glow brighter than status text or disclosure icons.

### Required fidelity surfaces

- Fonts and typography: unchanged; existing Inter/system and monospace hierarchy is preserved.
- Spacing and layout rhythm: the rail and node track moved onto the summary mark's centerline; row rhythm and disclosure spacing remain intact.
- Colors and visual tokens: semantic state colors reuse `--green`, `--cyan`, `--amber`, and `--red`; glow opacity stays below 30%.
- Image quality and asset fidelity: no raster assets, logos, or icons were replaced or approximated.
- Copy and content: unchanged.

### Comparison history

- Earlier P2 finding: the rail and nodes were 26 CSS px left of the Colossus summary mark, making the run header and episode timeline read as separate columns.
- Fix: moved the desktop rail/content geometry together and added a compact-breakpoint geometry that preserves the same centerline.
- Post-fix evidence: rail-to-node delta is 0 px and mark-to-rail delta is 0.5 px at both tested viewport widths.
- Earlier P3 polish: every neutral node looked equally weighted, so completed work and in-progress work were difficult to scan.
- Fix: added subtle semantic marker and run-mark glows for completed and active states while retaining amber waiting and red failure treatments.

### Interaction and validation checks

- Working Thread and Capsule disclosures remain interactive.
- The completed and active tool-state classes are covered by component tests.
- Desktop typecheck, all 173 renderer tests, 36 security-contract tests, and formatting checks pass.

## Aside canonical context, resize, and timeline polish — 2026-08-15

### Source and rendered evidence

- Source visual truth: `/var/folders/v5/10wplgc941b7wgrx5yvd64540000gp/T/codex-clipboard-3a294741-f2bc-4170-973a-46e938d38485.png` (2066 × 1652 px).
- Browser-rendered implementation: `/Users/alex/tools/Colossus/design-qa-aside-resize.png` (1280 × 720 px).
- State: a production-shaped Working Thread with an open Aside, a visible context cue, an empty Aside conversation, a persistent composer, and the new split handle. The smaller verification viewport intentionally differs from the source capture, so responsive behavior and minimum thread width were checked instead of matching absolute pane dimensions.

### Full-view comparison evidence

- The Aside remains visually attached to the source thread while gaining a narrow, keyboard-accessible resize separator. It retains the current Colossus border, spacing, and typography system.
- The Aside composer remains docked and visible while the conversation scrolls independently.
- The temporal rail and nodes share one exact integer centerline. Completed and active markers retain semantic color with a lower-opacity, shorter-radius halo than the preceding implementation.

### Context and interaction evidence

- Highlighted content now opens an Aside by source run identity only. The native runtime resolves the last canonical message for that run, so the visible final assistant response is included even when it arrived as a run result rather than a renderer activity event.
- The branch includes released user and assistant messages and deliberately omits tool calls and tool results. The renderer cannot choose a transcript boundary or submit another Space's context.
- Pointer dragging resizes the Aside within a 280–760 px bound while preserving at least 390 px for the source thread. Arrow keys, Shift+Arrow, Home, End, and double-click reset are supported through the accessible separator, and width persists locally.

### Required fidelity surfaces

- Fonts, icons, color tokens, copy hierarchy, and card geometry remain on the existing Desktop design system.
- No new raster assets, synthetic reasoning, or hidden tool output were introduced.
- The only visible changes are the adjustable split, corrected marker centerline, and restrained status glow.

### Comparison history

- P1 context failure: the renderer guessed a branch boundary from visible activity messages, excluding a final assistant response released through `RunResult.output`. Fixed by resolving the boundary from canonical native session history by source run ID.
- P2 pane usability: the fixed half-width Aside could crowd either surface. Fixed with a bounded, persistent, accessible split handle.
- P2 alignment: the rail and nodes used fractional/offset geometry. Fixed by placing the rail, summary mark, and episode nodes on one integer centerline.
- P3 polish: completed markers were brighter than the requested subtle treatment. Fixed by reducing border opacity, halo opacity, and blur radius.

### Validation checks

- Native context inclusion and tool-trace omission are covered by API-runtime and runtime tests.
- Resize bounds, defaults, persistence helpers, source-run-only renderer input, and accessible rendering are covered by renderer tests.
- Browser inspection confirmed the split handle, visible Aside composer, responsive thread/Aside layout, and corrected timeline geometry without a visible error surface.

## Empty-thread repo starter and sparkle state — 2026-08-15

### Source and rendered evidence

- Source visual truth: `/var/folders/v5/10wplgc941b7wgrx5yvd64540000gp/T/codex-clipboard-102bf62d-4948-4870-9acd-783ef5825639.png` (1498 × 416 px).
- Browser-rendered default state: `/Users/alex/tools/Colossus/design-qa-starters-default.png` (1280 × 720 px).
- Browser-rendered hover state: `/Users/alex/tools/Colossus/design-qa-starters-hover.png` (1280 × 720 px).
- CSS viewport: 1280 × 720. The source is a focused crop of the prompt region; the implementation capture includes the surrounding production-shaped Desktop shell, so comparison was normalized to the three-card region rather than absolute canvas position.
- State: connected Space, new empty thread, example prompts visible; the hover capture places the pointer over the first card's sparkle icon.

### Full-view and focused comparison evidence

- The implementation preserves the source's three-row stack, card order, border radius, spacing, dark surface, muted text, and left-aligned sparkle treatment.
- The requested copy change is intentional: the first card now reads `Orient yourself in this repo`; the other two prompts remain unchanged.
- Default icon color remains subordinate at `rgb(127, 154, 184)`. Hover raises it to `rgb(155, 212, 255)` with a bounded 5 px blue drop shadow, while the card receives the existing hover border/background treatment.
- No separate tighter crop was needed because the icon and all three prompt labels remain legible in the 1280 × 720 implementation captures and the source itself is already a focused region.

### Required fidelity surfaces

- Fonts and typography: unchanged from the existing Desktop system; weight, size, and line height retain the source hierarchy.
- Spacing and layout rhythm: unchanged three-card grid with the same 7 px gap and 10 × 12 px card padding.
- Colors and visual tokens: existing card colors are retained; the new semantic hover highlight uses a restrained cyan already compatible with Colossus's dark palette.
- Image quality and asset fidelity: the existing icon-library sparkle and Colossus mark are preserved; no substitute or generated asset was introduced.
- Copy and content: only the first starter prompt changed, exactly matching the requested repo-orientation task.

### Interaction, accessibility, and validation

- Hover was exercised in the in-app browser and the computed icon color/filter changed to the highlighted state.
- Clicking the first starter populated the composer with `Orient yourself in this repo`.
- `:focus-visible` shares the same card and icon treatment for keyboard users.
- Browser logs contained no errors; only normal Vite connection and React development messages were present.
- First comparison found no actionable P0/P1/P2 mismatch, so no corrective QA iteration was required.

final result: passed
