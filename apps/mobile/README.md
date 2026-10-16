# Colossus Mobile

Native Tauri 2 human client for iOS and Android. It uses the existing Control Plane
HTTP API and shared `@colossus/ui` tokens, controls, conversation components, and
host platform icons. The existing Desktop application remains the macOS product.

Install dependencies and check the initial client:

```sh
npm ci --ignore-scripts --legacy-peer-deps --install-links
npm run check
npm run dev
```

The development server offers **Explore development preview**, with an explicit
sample-data banner. `?fixture=login` exercises login presentation without real
credentials. Neither fixture is included in production builds. The browser preview
does not authenticate to a real controller; installed apps use native transport.

For browser tests, install Playwright Chromium and run `npm run test:browser`.
`PLAYWRIGHT_CHROMIUM_EXECUTABLE_PATH` may select an already installed test Chromium.
Screenshots are written to ignored `test-results/`.

From the repository root, `cargo xtask check mobile` checks the renderer, wrapper
formatting, and native session-client tests. See the [mobile development guide](../../docs/develop/mobile.md)
for native SDK setup, supported login behavior, infrastructure, and publication.

Work includes separate conversation and task inventories. Task details use the existing
bounded retained-event API, show released output, and link back to the associated
conversation. Cancellation is disabled until a native run has been allocated.
The browser suite covers password visibility/clearing, Large text and palettes, task
navigation, cancellation availability, and paginated output reconciliation. Sample
messages and tasks remain confined to the explicitly labeled development preview.
