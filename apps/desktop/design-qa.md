# Interaction Question Design QA

## Evidence

- Current source visual truth:
  `/var/folders/v5/10wplgc941b7wgrx5yvd64540000gp/T/codex-clipboard-228687ba-022b-4353-a20c-2d92eda4d94a.png`
- Current browser-rendered implementation:
  `/private/tmp/colossus-interaction-question-aligned.png`
- Current full-view comparison:
  `/private/tmp/colossus-interaction-question-alignment-comparison.png`
- Earlier compact-layout source:
  `/var/folders/v5/10wplgc941b7wgrx5yvd64540000gp/T/codex-clipboard-2ec13c76-e862-4f19-ae81-823a8f83cf82.png`
- Earlier desktop implementation:
  `/private/tmp/colossus-question-desktop.png`
- Selected desktop state:
  `/private/tmp/colossus-question-desktop-selected.png`
- Narrow implementation:
  `/private/tmp/colossus-question-mobile.png`
- Focused before/after comparison:
  `/private/tmp/colossus-question-comparison.png`
- State: pending `user_prompt`; the current source shows a selected six-choice
  question and the deterministic fixture shows the unselected four-choice state.
  The current comparison is scoped to the shared card/composer edge alignment, whose
  geometry is independent of question copy and selection state.
- Current implementation viewport and pixels: 900 × 940 CSS pixels at device-pixel
  ratio 1.
- Current source pixels: 1794 × 870; the screenshot is a double-density app crop.
- Desktop viewport: 1726 × 768 CSS pixels; browser device pixel ratio 2.
- Narrow viewport: 480 × 800 CSS pixels.
- Source pixels: 1726 × 736. The source card was a double-density crop, so its
  1480 × 502 pixel region was normalized to 740 × 251 for the focused comparison.
- Desktop implementation pixels: 1726 × 768. The rendered question card measured
  740 × 211.5 CSS pixels; its 740 × 212 crop was compared at 1:1.
- Narrow implementation pixels: 480 × 800. The card measured 464 × 297.5 CSS
  pixels.

## Findings

No actionable P0, P1, or P2 findings remain.

- Fonts and typography: the existing Inter/system stack, hierarchy, weights, and
  line heights remain consistent with the Desktop visual language. Long prompt text
  can wrap without displacing the response footer.
- Spacing and layout rhythm: the desktop choices use a balanced 2 × 2 grid and the
  card is 39.5 CSS pixels shorter than the normalized source. Header, answer body,
  and footer are visually distinct. At 480px the choices collapse to one column.
- Colors and visual tokens: existing surface, border, blue, amber, muted-text, and
  focus tokens are preserved. The selected choice has a clearly visible blue border,
  background, and native radio state.
- Card/composer alignment: the source card ended 50 CSS pixels before the composer's
  right edge because the generic interaction card retained its 740px cap inside a
  790px dock. The docked card now measures exactly the same left edge, right edge,
  and width as the composer.
- Image quality and asset fidelity: this component contains no raster imagery or
  custom image assets. Existing iconography elsewhere in the screen is unchanged.
- Copy and content: the original question and choice labels are preserved. Supporting
  copy is concise: “Select one response” changes to “Ready to send” after selection.

## Interaction And Accessibility Checks

- All four choices fit without scroll at 1726 × 768: answer body client height and
  scroll height both measured 101px.
- All four choices fit without scroll at 480 × 800: answer body client height and
  scroll height both measured 187px.
- Selecting “Rust” set the native radio checked state, changed the guidance to
  “Ready to send,” and enabled the response button.
- Sending the fixture response removed the pending interaction card.
- Non-respondable interactions disable every choice and the submit action in unit
  coverage.
- Browser console check returned no warnings or errors.
- Current browser geometry: before the fix the card measured 740px wide while its
  dock and composer measured 788.40625px. After the fix all three measure
  788.40625px, with matching 91.796875px left and 880.203125px right edges.
- Selecting “Rust” in the current fixture updated the guidance to “Ready to send”
  and enabled the response button.

## Comparison History

### Iteration 1

- Earlier finding: the global text-input rule inflated radio controls, the wide card
  used a single choice column, and the entire dock could scroll the action out of view.
- Fixes made: scoped native radio sizing, full-row choice targets, a two-column desktop
  grid, a one-column narrow layout, and fixed header/footer tracks around a bounded
  answer body.
- Post-fix evidence: `/private/tmp/colossus-question-comparison.png`,
  `/private/tmp/colossus-question-desktop-selected.png`, and
  `/private/tmp/colossus-question-mobile.png`.
- Result: all P1/P2 usability and density issues are resolved.

### Iteration 2

- Earlier finding: the docked response card inherited the generic 740px maximum
  width while the composer used the dock's 790px maximum, leaving a 50px shortfall
  on the card's right edge.
- Fix made: docked interaction cards now fill the pending-interaction dock and remove
  the generic card maximum.
- Post-fix evidence:
  `/private/tmp/colossus-interaction-question-alignment-comparison.png` and
  `/private/tmp/colossus-interaction-question-aligned.png`.
- Regression coverage: the browser acceptance test compares both left and right
  bounding-box edges at the supported 880 × 640 minimum viewport.
- Result: the response card and prompt composer share identical horizontal bounds;
  no P0/P1/P2 alignment findings remain.

Focused region comparison was required because the source screenshot was cropped around
the pending question rather than showing the complete application. Full-view evidence
was still used to verify the card’s relationship to the feed and disabled composer.

final result: passed
