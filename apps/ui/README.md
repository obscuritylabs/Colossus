# Colossus shared UI

`@colossus/ui` is a private source package consumed by Desktop, the VS Code
webviews, and the web control plane from the same checkout. It owns theme tokens
(Colossus blue, light, neutral dark, Black, and Hacker), the brand mark, application and
settings frames, compact catalog inventories, shadcn foundations, data tables, composer sizing and
keyboard behavior, and the accessible dropdown control. Each host compiles this
source into its own renderer bundle; this package is not independently
published or versioned for release.

Desktop keeps Tauri commands, appearance persistence, workspace/session orchestration,
queues, plugins, and capability decisions. VS Code keeps its native extension host,
worker SDK, credential handling, validated message protocol, and view projections.
Shared components take presentation props and callbacks only. They cannot import a
host bridge or transport. The web host keeps browser appearance persistence, project
state, authentication, and control-plane requests; the Rust cloud server owns authorization and runtime connections.

The app build and lockfiles remain separate. After changing this package, refresh
all local file dependencies with `npm ci --ignore-scripts` in `apps/desktop`,
`apps/vscode`, and `apps/web`, then run their `check`, `build`, and affected browser
checks. CI installs the shared source afresh and selects the host gates when
`apps/ui` changes.
No generated UI bundle needs committing. React is pinned to the same peer version
in all consumers so each renderer has one React instance.

Neutral dark is the VS Code default. Desktop keeps Colossus blue by default and
offers neutral dark, Black, and Hacker in Global → Appearance → Dark palette. Desktop keeps saved
custom palette colors while a preset palette is selected. Black uses shadcn’s near-black neutral surfaces
with Colossus accents; it is available in all three apps. Hacker translates
the TUI palette from `crates/colossus-presentation/src/palette.rs`: green prompts
and success, pale-green messages, cyan tools, amber warnings, and red errors.
The shared CSS adds near-black graphical surfaces and retains Colossus typography
and branding. It is selectable in all three apps; preferences remain local to each host.
Light and high contrast keep their respective token families. The native activity bar, left workspace view, and
right chat view stay owned by VS Code.

## shadcn foundations

New foundations live in `src/components/ui`, with a `radix-nova` shadcn CLI
configuration, Tabler icons, Radix composition, and Tailwind v4 utilities. Colossus
owns the component source and its styling. Buttons and inputs retain native HTML
behavior for ordinary actions; composed buttons and menus use Radix. Existing
`Button` variants remain compatible with the compact settings controls.

Import larger components through their own entry point:

```tsx
import { DataTable, type DataTableColumn } from "@colossus/ui/data-table";
```

This keeps table and menu dependencies out of hosts that do not use them. The table
supports sorting, search, column filters, pagination, and column visibility. Hosts
supply records, row IDs, cells, filter predicates, and action callbacks. It preserves
the current page during live refresh, clamps pages when records disappear, and resets
pagination when filters change. Key the component by project or inventory scope.
Server cursors and history loading remain host decisions; client sorting and filters
apply to the loaded records, so hosts must explain when more history is available.

`styles/shadcn.css` compiles `ui:` utilities from shared source only, without Tailwind
Preflight. Its inline token aliases reuse the existing surface, color, typography,
radius, and focus tokens. Avoid literal palette or text-size utilities in generated
components. Vite hosts use the Tailwind plugin; VS Code compiles the same stylesheet
with the pinned Node compiler and Oxide scanner and serves it under its existing CSP. Import the shared stylesheet
in a new renderer alongside `theme.css` and `select.css`.

Run the pinned CLI from the repository root to inspect the foundation:

```bash
npx shadcn@4.21.2 info --cwd apps/ui
```

Add future components in `apps/ui`, then review generated imports, styles, and new
dependencies against these boundaries. Keep adapters relative within shared source;
refresh all consumer lockfiles when dependency declarations change. Do not overwrite
Colossus token adaptations while updating upstream components. The retained MIT
license is in `assets/shadcn-LICENSE.txt` and accompanies renderer builds.

## Shared conversations

Import conversation components from `@colossus/ui/conversation` to keep Markdown
parser dependencies outside unrelated settings bundles. `ConversationEntry` owns
message geometry, avatar, typography, timestamps, and bounded Markdown rendering.
`ConversationTimeline` provides a common feed container. Hosts supply only released
content and retain event cursors, ordering, scrolling, tools, and interaction authority.

`ConversationComposer` is a controlled composer with context, controls, queue, notice,
and stop callbacks. Rich native editors can use `ConversationComposerFrame` around
host inputs and controls without replacing attachments, dictation, workspace state,
or submission logic. Buttons inside a composer form must declare their intended type.
The shared frame never selects models, grants capabilities, invokes tools, or persists
a draft. Send shortcuts and autosizing reuse the existing shared composer behavior.

`MarkdownContent` sanitizes generated content, omits remote images, and bounds parser
work by character count, tree depth, and node count. Links are inert by default; hosts
can supply a `linkComponent` that validates navigation through their own authority.
Desktop uses its browser controller. VS Code keeps generated links inert. Shared
styles respond to the same palette and text-size preferences as settings controls.

`ControlPlaneFrame` uses owned shadcn sidebar, menu, sheet, tooltip, collapsible,
and resizable foundations. Import `styles/control-plane.css` in the web host; its
workbench styles stay out of Desktop and editor bundles. Shared controls retain
native behavior and the existing palette and text-size tokens. Add future shadcn
components to this package as they are adopted, with relative internal imports,
exact Radix dependencies, and no persistence or transport in the foundation.

`ControlPlaneFrame` supplies the web primary rail, an optional scoped sidebar, content
header, and configurable classification banner. Navigation, authentication, projects,
and settings remain web/server decisions. It exposes no transport or persistence.

## Charts

Import charts through `@colossus/ui/charts`. The owned shadcn chart foundation
composes Recharts v3 with scoped semantic color tokens, tooltip content, and legends;
see the [official shadcn chart documentation](https://ui.shadcn.com/docs/components/radix/chart).
`ActivityLineChart` uses linear series for supplied daily UTC outcomes. It never fills
missing days or turns unknown values into zero, and exposes the same data in a table.
Its accessible Recharts layer supports keyboard inspection. Date-window selection,
authorization, querying, and coverage remain host decisions. The separate entry point
keeps chart code out of Desktop and editor renderers that do not use it.

## Automation screens

Import reusable workflow and schedule presentation from `@colossus/ui/automations`
and its stylesheet from `@colossus/ui/styles/automations.css`. `AutomationSurface`,
`AutomationWelcome`, `AutomationExampleGallery`, `AutomationInventory`, and
`AutomationOverview` share the same tokens in Desktop and Web. The inventory uses
the owned shadcn table, input, badge, and button foundations. Hosts provide released
records, labels, icons, disabled states, and callbacks; they retain prompts, timing
conversion, pagination, thread creation, execution, and capability decisions.

Workflow and schedule screens, review dialogs, typed input controls, calendar timing,
logic diagrams, and run-history presentation live under `src/automations`. Import
individual screens through `@colossus/ui/automations/WorkflowsSurface` or
`@colossus/ui/automations/SchedulesSurface`, and wrap them in `WorkflowHostProvider`.
The host adapter owns authenticated operations, selection epochs, capability facts,
model-profile discovery, and unknown-outcome classification. Shared forms retain a
reviewed immutable request and its retry identity; they never retry a mutation
implicitly. Desktop supplies its native bridge, and Web supplies project/node-scoped
controller requests.

Session headers, status presentation, seven session tabs, resource views, and topology
presentation are shared too. Hosts supply released run/resource models and link
navigation. A missing resource API stays explicitly unavailable; shared presentation
does not infer private context from assistant prose. The Artifact Library renders
released metadata supplied by its host. The appearance screen takes controlled values
and callbacks; storage, system-theme observation, and permission choices remain local
to each application. Palette parsing and contrast validation are shared foundations.

`ResearchControls` shares Desktop’s depth and evidence-source presentation. Hosts
supply selected values, callbacks, and available lanes. Web obtains Research support
from the selected runtime’s authenticated capability response, fences it to that
connection, and intersects evidence choices with the reported tool ceiling. Missing
support never changes the user’s requested mode into Execute.

`GoalControls` shares the bounded iteration input between Desktop and Web. Hosts
offer standalone Goal mode only after authenticated `goal.create` discovery, capture
its 1–50 iteration budget in the durable run request, and retain queue, permission,
and cancellation decisions. A Plan is not required.
