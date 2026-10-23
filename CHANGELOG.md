# Changelog

All notable changes to Colossus will be documented in this file.

This project follows the spirit of [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and uses semantic versioning before 1.0 with the usual caveat that minor versions may
include breaking changes while the public API is still settling.

## [Unreleased]

## [0.11.11-preview.2] - 2026-10-23

### Highlights

- Control Plane and Project administration use compact entry points with scoped secondary navigation.
- Settings forms have shareable create and edit URLs, preserve drafts after failed saves, and guard unsaved navigation.
- Instance provider, model, MCP, Search, and variable definitions flow into project settings with explicit override and credential assignment boundaries.
- User administration, instance announcements, classification colors, and collapsible pinned chats are available in Hosted Web.
- Newly created projects can inspect inherited settings before an operator enables hosted execution.
- Global connection definitions publish independently of project restrictions; projects default to allowing configured connections and can manage exact-origin restrictions in Network access.
- Hosted settings validate labels, model limits, connection fields and MCP selectors before saving, with aligned forms, save toasts and compact credential refresh controls.
- Credentialed loopback connections follow their saved transport settings, and invalid connection bindings are rejected before work is queued.
- Control Plane server bundles include archive-specific guidance, with packaging inputs checked before release builds.
- Update ctutils to 0.4.4 to address RUSTSEC-2026-0385.
- Update the Web and Desktop test runner to Vitest 4.1.12 for the current development-server security fixes.

<!-- colossus:release-notes base=v0.11.10 -->

### Merged changes

#### Added

- **control-plane:** replace Kustomize with a Helm chart ([2cf71f0](https://github.com/obscuritylabs/Colossus/commit/2cf71f0823f66742004f9b0a30fd121caae72bc8))
- **cloud:** add instance announcements and classification colors ([e78b2e4](https://github.com/obscuritylabs/Colossus/commit/e78b2e4678685b69d481abdfa2cfe5be89c79932))
- **web:** add pinned hosted chats ([013a535](https://github.com/obscuritylabs/Colossus/commit/013a535f5a6d3968fa508573d5717b1d11384599))
- **web:** make pinned chats collapsible ([021b742](https://github.com/obscuritylabs/Colossus/commit/021b7421aea37c970a7fa257ad88faba52fb678b))
- **cloud:** add scoped settings and user administration ([e1754b3](https://github.com/obscuritylabs/Colossus/commit/e1754b307efb2b6d91e459564a5cf7ffcc9199c6))
- **web:** add scoped administration and shareable settings forms ([2ed8b2e](https://github.com/obscuritylabs/Colossus/commit/2ed8b2e0df96dc29c8517e154cf48941d6a6caed))

#### Fixed

- **browser:** bound owner lease watcher shutdown ([dfd2e2c](https://github.com/obscuritylabs/Colossus/commit/dfd2e2c036185dc00279462edecb52e7623fc0c6))
- **browser:** poll owner leases for bounded retirement ([61d1f59](https://github.com/obscuritylabs/Colossus/commit/61d1f597535dda7525d3d46b8fd5d61afbef186b))
- **release:** retain usable Control Plane bundle guidance ([8d20776](https://github.com/obscuritylabs/Colossus/commit/8d20776265f2cbec7c6065870ee9eafab2330ff1))
- **cloud:** make inherited connections usable across scopes ([129114c](https://github.com/obscuritylabs/Colossus/commit/129114c39c838d9f7f418f05495fc791f65726f5))
- **ui:** validate scoped settings and improve save feedback ([5c4bec3](https://github.com/obscuritylabs/Colossus/commit/5c4bec31ffdb49d71b1c8d5a96cbcbd9160355ec))

#### Security

- **deps:** update ctutils to patched 0.4.4 ([a7cb848](https://github.com/obscuritylabs/Colossus/commit/a7cb848067e68fc8f065b7de0afab10fe44b4036))
- **ui:** update Vitest and isolate grammar contract cases ([32df279](https://github.com/obscuritylabs/Colossus/commit/32df279a2a8e065c1542258581900e8d3dea76d7))

#### Maintenance

- **packaging:** pin Homebrew and Nix to 0.11.10 ([#314](https://github.com/obscuritylabs/Colossus/pull/314))
- **release:** prepare 0.11.11-preview.1 ([e5d2ed2](https://github.com/obscuritylabs/Colossus/commit/e5d2ed2df4712bd4edfbab9f7810a13681340fa2))
- **release:** prepare 0.11.11-preview.2 ([625338d](https://github.com/obscuritylabs/Colossus/commit/625338de8a8cfc4c98470db2251dcb0e2eb90d95))
- **release:** include preview security fix in notes ([5a42de4](https://github.com/obscuritylabs/Colossus/commit/5a42de4c1623a4f7ba0a06fe2c648657c71ef35d))

**Source:** [5c4bec3](https://github.com/obscuritylabs/Colossus/commit/5c4bec31ffdb49d71b1c8d5a96cbcbd9160355ec)

**Full changelog:** [v0.11.10...v0.11.11-preview.2](https://github.com/obscuritylabs/Colossus/compare/v0.11.10...v0.11.11-preview.2)

<!-- /colossus:release-notes -->

## [0.11.11-preview.1] - 2026-10-23

### Highlights

- Control Plane and Project administration use compact entry points with scoped secondary navigation.
- Settings forms have shareable create and edit URLs, preserve drafts after failed saves, and guard unsaved navigation.
- Instance provider, model, MCP, Search, and variable definitions flow into project settings with explicit override and credential assignment boundaries.
- User administration, instance announcements, classification colors, and collapsible pinned chats are available in Hosted Web.
- Newly created projects can inspect inherited settings before an operator enables hosted execution.

<!-- colossus:release-notes base=v0.11.10 -->

### Merged changes

#### Added

- **control-plane:** replace Kustomize with a Helm chart ([2cf71f0](https://github.com/obscuritylabs/Colossus/commit/2cf71f0823f66742004f9b0a30fd121caae72bc8))
- **cloud:** add instance announcements and classification colors ([e78b2e4](https://github.com/obscuritylabs/Colossus/commit/e78b2e4678685b69d481abdfa2cfe5be89c79932))
- **web:** add pinned hosted chats ([013a535](https://github.com/obscuritylabs/Colossus/commit/013a535f5a6d3968fa508573d5717b1d11384599))
- **web:** make pinned chats collapsible ([021b742](https://github.com/obscuritylabs/Colossus/commit/021b7421aea37c970a7fa257ad88faba52fb678b))
- **cloud:** add scoped settings and user administration ([e1754b3](https://github.com/obscuritylabs/Colossus/commit/e1754b307efb2b6d91e459564a5cf7ffcc9199c6))
- **web:** add scoped administration and shareable settings forms ([2ed8b2e](https://github.com/obscuritylabs/Colossus/commit/2ed8b2e0df96dc29c8517e154cf48941d6a6caed))

#### Fixed

- **browser:** bound owner lease watcher shutdown ([dfd2e2c](https://github.com/obscuritylabs/Colossus/commit/dfd2e2c036185dc00279462edecb52e7623fc0c6))
- **browser:** poll owner leases for bounded retirement ([61d1f59](https://github.com/obscuritylabs/Colossus/commit/61d1f597535dda7525d3d46b8fd5d61afbef186b))

#### Maintenance

- **packaging:** pin Homebrew and Nix to 0.11.10 ([#314](https://github.com/obscuritylabs/Colossus/pull/314))

**Source:** [2ed8b2e](https://github.com/obscuritylabs/Colossus/commit/2ed8b2e0df96dc29c8517e154cf48941d6a6caed)

**Full changelog:** [v0.11.10...v0.11.11-preview.1](https://github.com/obscuritylabs/Colossus/compare/v0.11.10...v0.11.11-preview.1)

<!-- /colossus:release-notes -->

## [0.11.10] - 2026-10-22

### Highlights

- Hosted Web adds personal chats, separate agent management, private provider settings,
  forks and side chats, with durable recovery across shared deployments.
- Web and Desktop share compact navigation, aligned chat workspaces, resizable tabbed
  inspectors and cleaner plan, activity, snapshot, source and resource views.
- Web, Desktop and VS Code share streaming Markdown, syntax highlighting, code and
  message copying, reported run usage and offline Mermaid previews.
- Session titles update consistently across hosts and survive recovery without
  replacing manually chosen names.
- Windows Desktop reviews client-certificate choices inside the application while
  keeping private keys in native custody.

<!-- colossus:release-notes base=v0.11.8 -->

### Merged changes

#### Added

- **http:** add bounded redirects and Desktop settings ([#289](https://github.com/obscuritylabs/Colossus/pull/289))
- **ui:** unify web workspace management and standalone Goal mode ([#287](https://github.com/obscuritylabs/Colossus/pull/287))
- **agents:** add durable inboxes and A2A application edge ([#288](https://github.com/obscuritylabs/Colossus/pull/288))
- **models:** switch models and reasoning in Desktop, VS Code and TUI ([#295](https://github.com/obscuritylabs/Colossus/pull/295))
- **provider:** add native Anthropic Messages support ([#297](https://github.com/obscuritylabs/Colossus/pull/297))
- **vscode:** bundle and manage local runtimes in the sidebar ([#296](https://github.com/obscuritylabs/Colossus/pull/296))
- **images:** add local image viewing and activity previews ([#298](https://github.com/obscuritylabs/Colossus/pull/298))
- **browser:** connect owned Chromium hosts to Core and Desktop ([#290](https://github.com/obscuritylabs/Colossus/pull/290))
- **browser:** add runtime settings and bundled browser guidance ([637e25c](https://github.com/obscuritylabs/Colossus/commit/637e25ce8e326fb1c9b8e25d0c8ea3f7bd00cd71))
- **computer:** add opt-in Windows desktop control ([c229164](https://github.com/obscuritylabs/Colossus/commit/c229164951faf8203c309dada2f1b6249935908b))
- **windows:** expose read-only process job accounting ([c3e4a41](https://github.com/obscuritylabs/Colossus/commit/c3e4a4194b3729a3554b3a83a5989443d2273692))
- **windows:** expose read-only process job accounting ([8c3d266](https://github.com/obscuritylabs/Colossus/commit/8c3d266f84c8967a29c800430f5e6985b795ef20))
- **browser:** package Windows preview and preserve agent views ([c8d82a8](https://github.com/obscuritylabs/Colossus/commit/c8d82a825f4a2dd29bcbf99b30e7604f8b23a97a))
- **browser:** package Windows preview and preserve agent views ([004ea37](https://github.com/obscuritylabs/Colossus/commit/004ea37e916574ebe19438d5777e0289dd87e959))
- **cloud:** separate hosted execution and add web agent management ([ad9e249](https://github.com/obscuritylabs/Colossus/commit/ad9e249d9da66dd4852daacb0b95af746d6ab909))
- **chat:** unify hosted workspaces and shared message rendering ([9afaf49](https://github.com/obscuritylabs/Colossus/commit/9afaf49d7fbd6f72d68fbfe384a09230350c3ca7))

#### Fixed

- **plugins:** avoid OCI packaging for local snapshots ([#293](https://github.com/obscuritylabs/Colossus/pull/293))
- **desktop:** apply access settings without redundant prompts ([#294](https://github.com/obscuritylabs/Colossus/pull/294))
- **browser:** preserve input gestures and agent viewer continuity ([b6c57f2](https://github.com/obscuritylabs/Colossus/commit/b6c57f23ddb3438253066e65bac75e4be0f33dd1))
- **browser:** polish capability settings presentation ([492935a](https://github.com/obscuritylabs/Colossus/commit/492935a47ae59728e106512a82858f6b57bd3b8e))
- **browser:** retain focus after inheriting settings ([5b39d62](https://github.com/obscuritylabs/Colossus/commit/5b39d62ff8597d71ffd4d9b963cc0a6e248c5904))
- **browser:** load public pages and reveal agent tabs ([9989d72](https://github.com/obscuritylabs/Colossus/commit/9989d72d8e93150cd8b807b538c5c0d17f2508c0))
- **desktop:** preserve browser views and clarify agent settings ([88cc3d1](https://github.com/obscuritylabs/Colossus/commit/88cc3d1847d27e08972b005a5bf1a4a250239dbe))
- **desktop:** harden credential labels and native acceptance fixtures ([b3cc106](https://github.com/obscuritylabs/Colossus/commit/b3cc106fcce290adbc229e014117a772e3a2f584))
- **desktop:** upgrade Tao to prevent Windows keyboard deadlocks ([e9a2b80](https://github.com/obscuritylabs/Colossus/commit/e9a2b8061f0e82cde8cb9fccdc15767fedb424bc))
- **browser:** retain completed views and show agent control ([7111d76](https://github.com/obscuritylabs/Colossus/commit/7111d768d954309054ea7afef776571291d2f464))
- **browser:** retain acknowledged leases during viewport recovery ([54ab8f3](https://github.com/obscuritylabs/Colossus/commit/54ab8f3759a7d7c5094c7620a1c4e0afa7e0607f))
- **browser:** retire expired network sessions with clear recovery ([46444f4](https://github.com/obscuritylabs/Colossus/commit/46444f44c298c9bf575d747c69606e3516e68492))
- **codex:** update protocol and retain stream error diagnostics ([4e63bed](https://github.com/obscuritylabs/Colossus/commit/4e63bed240ec22a8ab1a3cb4f10b930cb48524c9))
- **computer:** retain verified Windows root for capture helpers ([6a5a730](https://github.com/obscuritylabs/Colossus/commit/6a5a7307fb05cc790ae41aa46521dfa040b05eb5))
- **desktop:** include computer tools in managed application grants ([1dab1a4](https://github.com/obscuritylabs/Colossus/commit/1dab1a4d316e4ccbf5fcf17911d9a7ca8a1de128))
- **tools:** accept attached search term prefixes ([3337a92](https://github.com/obscuritylabs/Colossus/commit/3337a924de5a408b6816837c1f726b7b28c5bc87))
- **computer:** settle foreground activation and preserve input markers ([fcd03c5](https://github.com/obscuritylabs/Colossus/commit/fcd03c5f67e4be4ed8c9da77d52f261055400b6b))
- **browser:** allow bounded slow metadata reads without disconnecting ([2a4c18a](https://github.com/obscuritylabs/Colossus/commit/2a4c18a1a1400410c7179a356df9892ab682261b))
- **browser:** allow bounded slow metadata reads without disconnecting ([055a7c8](https://github.com/obscuritylabs/Colossus/commit/055a7c815807307272ae011e3726e59342e45c3d))
- **desktop:** restore new-thread composer from inbox views ([2c824b4](https://github.com/obscuritylabs/Colossus/commit/2c824b49ce8c800d41c8ac6db4a2e4de6390f99a))
- **desktop:** restore new-thread composer from inbox views ([ca16256](https://github.com/obscuritylabs/Colossus/commit/ca162568c9c8b784b907c9590b526a6e45e817d9))
- **release:** recognize the Windows browser preview packager ([8e07db6](https://github.com/obscuritylabs/Colossus/commit/8e07db6a12eeca366e3af34799b3ac9e0198b01c))
- **release:** recognize the Windows browser preview packager ([164d178](https://github.com/obscuritylabs/Colossus/commit/164d17807d6911317d18568821b436011c934a8f))
- **build:** inline ARM64 musl C atomics for static linking ([26f13b9](https://github.com/obscuritylabs/Colossus/commit/26f13b9301180e1693fd1d668fbaa3f20337344b))
- **desktop:** keep native browser behind image previews ([523f54c](https://github.com/obscuritylabs/Colossus/commit/523f54cc0c33aad8bf366238b58c9c6c2b7da337))
- **computer:** enforce explicit operator control and explain failures ([a81071d](https://github.com/obscuritylabs/Colossus/commit/a81071d5503b4f89da5f37b37ed92b27c75ed923))
- **computer:** narrow requested read counts to configured limits ([15a6ee7](https://github.com/obscuritylabs/Colossus/commit/15a6ee7ffe529ff6556ea99a52840f8b1a07d05d))
- **provider:** project Anthropic-compatible tool schemas ([#301](https://github.com/obscuritylabs/Colossus/pull/301))
- **windows:** repair browser startup and computer focus guidance ([d714d59](https://github.com/obscuritylabs/Colossus/commit/d714d59f7d5c784b21ff9cdc5212a1e5adfb5ea8))
- **ci:** scope long browser paths to the runtime fixture ([9e94ffe](https://github.com/obscuritylabs/Colossus/commit/9e94ffe4d8fd5e6cd2c2f6398287ad5b77fde301))
- **browser:** apply terminal viewer revocation once ([02076cf](https://github.com/obscuritylabs/Colossus/commit/02076cfd6ef9ada866525b14f118a072f3b6390e))
- **browser:** retire viewers after explicit session cleanup ([41473f9](https://github.com/obscuritylabs/Colossus/commit/41473f92e690a439beb1efbce53d30eca35983a5))
- **browser:** preserve viewer recovery between sessions ([cd0a8d5](https://github.com/obscuritylabs/Colossus/commit/cd0a8d5dfa03233526a96aa2489ac03f29686138))
- **browser:** preserve visible views across superseded hides ([dbfeee0](https://github.com/obscuritylabs/Colossus/commit/dbfeee026554eebfffd910da7e119bd3fe759792))
- **grpc:** offload bounded artifact reads ([6a9c4f8](https://github.com/obscuritylabs/Colossus/commit/6a9c4f89b6bc5d40ac3689d0400cbefb142f9e45))
- **browser:** coalesce acknowledged hidden leases ([4df6676](https://github.com/obscuritylabs/Colossus/commit/4df6676ea8ff9960e5067d4f862bb015034e778f))
- resolve desktop testing regressions and add native logs ([dee9362](https://github.com/obscuritylabs/Colossus/commit/dee93622a2a8c1fe7518d3fb9112c859f3f514ec))
- propagate native navigation failures before observations ([0c1d39f](https://github.com/obscuritylabs/Colossus/commit/0c1d39f1e7c9ed314321cbdec924e12fabf01c83))
- reuse retained browser frame budgets on replacement ([a67388e](https://github.com/obscuritylabs/Colossus/commit/a67388e36c685c529cc5c6cc38c388ad6f807f37))
- bound tool context and add Windows client identity review ([40a8923](https://github.com/obscuritylabs/Colossus/commit/40a8923e5f1c0c28d2a2a3e8ddf637d0bd8552ec))
- **desktop:** resolve native validation and terminal capture races ([7e16b59](https://github.com/obscuritylabs/Colossus/commit/7e16b59af3205ade1cc280054ee66057b1e960fd))
- **hosted:** retain runtimes during shared MCP sign-in ([2f261b9](https://github.com/obscuritylabs/Colossus/commit/2f261b985cd3b7e7b93e2ce1e5840c8cf484e847))
- repair browser deadlines and Windows desktop indexing ([#309](https://github.com/obscuritylabs/Colossus/pull/309))
- **desktop:** keep PKI certificate selection inside the app ([#311](https://github.com/obscuritylabs/Colossus/pull/311))
- **vscode:** retain interactive inspector Markdown ([29f90da](https://github.com/obscuritylabs/Colossus/commit/29f90daecbe5cb64c4c461a8d3640bf9ff70f13c))
- **ui:** align clean installs and packaged avatar checks ([c5b3ba6](https://github.com/obscuritylabs/Colossus/commit/c5b3ba6bbd9d96038f8cadef4f88353130d7e59d))

#### Security

- **dependencies:** replace vulnerable desktop and credential backends ([3cea322](https://github.com/obscuritylabs/Colossus/commit/3cea3224e398e78848c2315a039f44b1f0201a46))
- **dependencies:** replace vulnerable desktop and credential backends ([dd3525a](https://github.com/obscuritylabs/Colossus/commit/dd3525ab58d1a17a6f74fb3db35cafb4455fa861))
- patch bundled trust certificates and document native acceptance ([dfb02de](https://github.com/obscuritylabs/Colossus/commit/dfb02de134b6e488e5ea26b0e5e43ab596fe8c7f))
- patch frontend dev server and retain enrollment contract ([f8075a4](https://github.com/obscuritylabs/Colossus/commit/f8075a45e75e2c7043396a961c92a63c09fa0c6c))

#### Changed

- **storage:** seek Redb cursors and batch projection catch-up ([#286](https://github.com/obscuritylabs/Colossus/pull/286))

#### Documentation

- **release:** prepare 0.11.8 preview 2 history ([6994da7](https://github.com/obscuritylabs/Colossus/commit/6994da76d12820007d973a56f2d83a04f3f08d47))

#### Maintenance

- **deps-dev:** update desktop and web Vitest to 4.1.11 ([#292](https://github.com/obscuritylabs/Colossus/pull/292))
- **browser:** import expiry test clock for native builds ([afa8a99](https://github.com/obscuritylabs/Colossus/commit/afa8a99e60f689e84e6dbc77c3088c143fc9be72))
- **computer:** trace native foreground eligibility ([e84d921](https://github.com/obscuritylabs/Colossus/commit/e84d9218cc411be051ebbab1daa8fea432147cb6))
- **computer:** trace native input terminal receipts ([83e1f6f](https://github.com/obscuritylabs/Colossus/commit/83e1f6f67605ad0044bbfaca4ceb4aa3e82b42c8))
- **computer:** trace native control admission refusals ([8572c0c](https://github.com/obscuritylabs/Colossus/commit/8572c0c30e379bcbe9fbc9406ca8f19e1acb169b))
- **desktop:** verify authenticated computer tool discovery ([0bb3b9e](https://github.com/obscuritylabs/Colossus/commit/0bb3b9e0d6fd86e5ec6ff7806af24f1bf02afeb0))
- **desktop:** align provider inventory credential copy ([7561476](https://github.com/obscuritylabs/Colossus/commit/7561476968911908b75a90e5f7e501724dccf581))
- **release:** allow cold Intel macOS packaging to finish ([54bf5d6](https://github.com/obscuritylabs/Colossus/commit/54bf5d61ce2f9fd3807791bb5b3dd970201930cb))
- **ci:** validate bounded matrix timeout branches ([23761f9](https://github.com/obscuritylabs/Colossus/commit/23761f9a4d88a9c0924c29e740d4b7c99293cdd3))
- allow cold Windows Desktop acceptance to finish ([0299cac](https://github.com/obscuritylabs/Colossus/commit/0299cac53c5dd19c0322cda04eeb3e6034761060))
- refresh Windows release signing credentials ([024f166](https://github.com/obscuritylabs/Colossus/commit/024f16604f24de71bbf75c890a2e57d969e1127a))
- **browser:** report Windows SDK close evidence accurately ([9f1a395](https://github.com/obscuritylabs/Colossus/commit/9f1a3954e2c4d39af7b4200a1780866196341040))
- **browser:** trace hidden viewport polling failures ([0517afa](https://github.com/obscuritylabs/Colossus/commit/0517afa1bff8dca6d7f1f470fe42c83f78072be9))
- **browser:** reproduce modal visibility traffic ([d943cc1](https://github.com/obscuritylabs/Colossus/commit/d943cc1b6e891ba8719a9b50e6082e9741f78527))
- **browser:** distinguish wire and read delays ([f6cff19](https://github.com/obscuritylabs/Colossus/commit/f6cff197ce8428172eaae063f969801e5a0b08d0))
- **grpc:** trace slow history list polls ([ddbd3ef](https://github.com/obscuritylabs/Colossus/commit/ddbd3ef994847fe1b7e5ec0fdd52693c2386971f))
- **release:** prepare 0.11.9 changelog ([61d3d68](https://github.com/obscuritylabs/Colossus/commit/61d3d68747a2019edc16d67f621ac790a9b15576))
- assert live profile restoration and completed tool calls ([09a1d1d](https://github.com/obscuritylabs/Colossus/commit/09a1d1d26fb652df058577cda4e9c9e2528dd226))
- **release:** refresh 0.11.9 validation notes ([e933585](https://github.com/obscuritylabs/Colossus/commit/e9335858e09707d8e230c71b1d6889010e45a2ce))
- keep Windows discovery fixture reads blocking ([c0d8f28](https://github.com/obscuritylabs/Colossus/commit/c0d8f28169277d146195edad7be8c634e1a557aa))
- decode current tool observations in browser fixtures ([7368d33](https://github.com/obscuritylabs/Colossus/commit/7368d33898ca1bf262c8be2a3e828212f8609fc9))
- fence viewport pulses before browser session replacement ([5b0b0e0](https://github.com/obscuritylabs/Colossus/commit/5b0b0e095de45efd0e9e4ab47716c2ff811fae79))
- **web:** wait for restored editors and navigation ([eef3c4e](https://github.com/obscuritylabs/Colossus/commit/eef3c4ee6ff9ccd73cefb3ffee386ade374bb93d))
- **desktop:** lock the shared cloud executor ([1dc8796](https://github.com/obscuritylabs/Colossus/commit/1dc8796ffbe2a8012e0286206b016ed1a94af8ca))
- **release:** prepare 0.11.10 stable history ([048ecc1](https://github.com/obscuritylabs/Colossus/commit/048ecc1a26ab133b6e0030e9a6498a630b1d34e9))
- **release:** include PKI and inspector fixes in 0.11.10 ([9f2579a](https://github.com/obscuritylabs/Colossus/commit/9f2579a5a8f1219f0d3aa7d2439260f4ae3da1da))

**Source:** [c5b3ba6](https://github.com/obscuritylabs/Colossus/commit/c5b3ba6bbd9d96038f8cadef4f88353130d7e59d)

**Full changelog:** [v0.11.8...v0.11.10](https://github.com/obscuritylabs/Colossus/compare/v0.11.8...v0.11.10)

<!-- /colossus:release-notes -->

## [0.11.9] - 2026-10-21

### Highlights

- Browser sessions retain their page between agent turns, resume with fresh control
  authority, and keep the last accepted view available for inspection.
- Agent context retires superseded browser and computer observations during a task
  and removes exact duplicate read results. Provider continuation respects protected
  thinking and retires saved Responses state when its observations become obsolete.
- Windows browser previews use system certificate trust and offer native, exact-origin
  approval for Windows client certificates. Private keys stay in the Windows store.
- Desktop and provider operations emit bounded, rotating local INFO logs; DEBUG is
  available through `COLOSSUS_LOG_LEVEL=debug`. Request bodies and credentials are excluded.
- Desktop access-profile recovery, capabilities inventory, provider enrollment,
  Anthropic response spacing, and terminal text selection are improved. The redundant
  Desktop Library destination is removed while session resources remain available.

<!-- colossus:release-notes base=v0.11.8 -->

### Merged changes

#### Added

- **http:** add bounded redirects and Desktop settings ([#289](https://github.com/obscuritylabs/Colossus/pull/289))
- **ui:** unify web workspace management and standalone Goal mode ([#287](https://github.com/obscuritylabs/Colossus/pull/287))
- **agents:** add durable inboxes and A2A application edge ([#288](https://github.com/obscuritylabs/Colossus/pull/288))
- **models:** switch models and reasoning in Desktop, VS Code and TUI ([#295](https://github.com/obscuritylabs/Colossus/pull/295))
- **provider:** add native Anthropic Messages support ([#297](https://github.com/obscuritylabs/Colossus/pull/297))
- **vscode:** bundle and manage local runtimes in the sidebar ([#296](https://github.com/obscuritylabs/Colossus/pull/296))
- **images:** add local image viewing and activity previews ([#298](https://github.com/obscuritylabs/Colossus/pull/298))
- **browser:** connect owned Chromium hosts to Core and Desktop ([#290](https://github.com/obscuritylabs/Colossus/pull/290))
- **browser:** add runtime settings and bundled browser guidance ([637e25c](https://github.com/obscuritylabs/Colossus/commit/637e25ce8e326fb1c9b8e25d0c8ea3f7bd00cd71))
- **computer:** add opt-in Windows desktop control ([c229164](https://github.com/obscuritylabs/Colossus/commit/c229164951faf8203c309dada2f1b6249935908b))
- **windows:** expose read-only process job accounting ([c3e4a41](https://github.com/obscuritylabs/Colossus/commit/c3e4a4194b3729a3554b3a83a5989443d2273692))
- **windows:** expose read-only process job accounting ([8c3d266](https://github.com/obscuritylabs/Colossus/commit/8c3d266f84c8967a29c800430f5e6985b795ef20))
- **browser:** package Windows preview and preserve agent views ([c8d82a8](https://github.com/obscuritylabs/Colossus/commit/c8d82a825f4a2dd29bcbf99b30e7604f8b23a97a))
- **browser:** package Windows preview and preserve agent views ([004ea37](https://github.com/obscuritylabs/Colossus/commit/004ea37e916574ebe19438d5777e0289dd87e959))

#### Fixed

- **plugins:** avoid OCI packaging for local snapshots ([#293](https://github.com/obscuritylabs/Colossus/pull/293))
- **desktop:** apply access settings without redundant prompts ([#294](https://github.com/obscuritylabs/Colossus/pull/294))
- **browser:** preserve input gestures and agent viewer continuity ([b6c57f2](https://github.com/obscuritylabs/Colossus/commit/b6c57f23ddb3438253066e65bac75e4be0f33dd1))
- **browser:** polish capability settings presentation ([492935a](https://github.com/obscuritylabs/Colossus/commit/492935a47ae59728e106512a82858f6b57bd3b8e))
- **browser:** retain focus after inheriting settings ([5b39d62](https://github.com/obscuritylabs/Colossus/commit/5b39d62ff8597d71ffd4d9b963cc0a6e248c5904))
- **browser:** load public pages and reveal agent tabs ([9989d72](https://github.com/obscuritylabs/Colossus/commit/9989d72d8e93150cd8b807b538c5c0d17f2508c0))
- **desktop:** preserve browser views and clarify agent settings ([88cc3d1](https://github.com/obscuritylabs/Colossus/commit/88cc3d1847d27e08972b005a5bf1a4a250239dbe))
- **desktop:** harden credential labels and native acceptance fixtures ([b3cc106](https://github.com/obscuritylabs/Colossus/commit/b3cc106fcce290adbc229e014117a772e3a2f584))
- **desktop:** upgrade Tao to prevent Windows keyboard deadlocks ([e9a2b80](https://github.com/obscuritylabs/Colossus/commit/e9a2b8061f0e82cde8cb9fccdc15767fedb424bc))
- **browser:** retain completed views and show agent control ([7111d76](https://github.com/obscuritylabs/Colossus/commit/7111d768d954309054ea7afef776571291d2f464))
- **browser:** retain acknowledged leases during viewport recovery ([54ab8f3](https://github.com/obscuritylabs/Colossus/commit/54ab8f3759a7d7c5094c7620a1c4e0afa7e0607f))
- **browser:** retire expired network sessions with clear recovery ([46444f4](https://github.com/obscuritylabs/Colossus/commit/46444f44c298c9bf575d747c69606e3516e68492))
- **codex:** update protocol and retain stream error diagnostics ([4e63bed](https://github.com/obscuritylabs/Colossus/commit/4e63bed240ec22a8ab1a3cb4f10b930cb48524c9))
- **computer:** retain verified Windows root for capture helpers ([6a5a730](https://github.com/obscuritylabs/Colossus/commit/6a5a7307fb05cc790ae41aa46521dfa040b05eb5))
- **desktop:** include computer tools in managed application grants ([1dab1a4](https://github.com/obscuritylabs/Colossus/commit/1dab1a4d316e4ccbf5fcf17911d9a7ca8a1de128))
- **tools:** accept attached search term prefixes ([3337a92](https://github.com/obscuritylabs/Colossus/commit/3337a924de5a408b6816837c1f726b7b28c5bc87))
- **computer:** settle foreground activation and preserve input markers ([fcd03c5](https://github.com/obscuritylabs/Colossus/commit/fcd03c5f67e4be4ed8c9da77d52f261055400b6b))
- **browser:** allow bounded slow metadata reads without disconnecting ([2a4c18a](https://github.com/obscuritylabs/Colossus/commit/2a4c18a1a1400410c7179a356df9892ab682261b))
- **browser:** allow bounded slow metadata reads without disconnecting ([055a7c8](https://github.com/obscuritylabs/Colossus/commit/055a7c815807307272ae011e3726e59342e45c3d))
- **desktop:** restore new-thread composer from inbox views ([2c824b4](https://github.com/obscuritylabs/Colossus/commit/2c824b49ce8c800d41c8ac6db4a2e4de6390f99a))
- **desktop:** restore new-thread composer from inbox views ([ca16256](https://github.com/obscuritylabs/Colossus/commit/ca162568c9c8b784b907c9590b526a6e45e817d9))
- **release:** recognize the Windows browser preview packager ([8e07db6](https://github.com/obscuritylabs/Colossus/commit/8e07db6a12eeca366e3af34799b3ac9e0198b01c))
- **release:** recognize the Windows browser preview packager ([164d178](https://github.com/obscuritylabs/Colossus/commit/164d17807d6911317d18568821b436011c934a8f))
- **build:** inline ARM64 musl C atomics for static linking ([26f13b9](https://github.com/obscuritylabs/Colossus/commit/26f13b9301180e1693fd1d668fbaa3f20337344b))
- **desktop:** keep native browser behind image previews ([523f54c](https://github.com/obscuritylabs/Colossus/commit/523f54cc0c33aad8bf366238b58c9c6c2b7da337))
- **computer:** enforce explicit operator control and explain failures ([a81071d](https://github.com/obscuritylabs/Colossus/commit/a81071d5503b4f89da5f37b37ed92b27c75ed923))
- **computer:** narrow requested read counts to configured limits ([15a6ee7](https://github.com/obscuritylabs/Colossus/commit/15a6ee7ffe529ff6556ea99a52840f8b1a07d05d))
- **provider:** project Anthropic-compatible tool schemas ([#301](https://github.com/obscuritylabs/Colossus/pull/301))
- **windows:** repair browser startup and computer focus guidance ([d714d59](https://github.com/obscuritylabs/Colossus/commit/d714d59f7d5c784b21ff9cdc5212a1e5adfb5ea8))
- **ci:** scope long browser paths to the runtime fixture ([9e94ffe](https://github.com/obscuritylabs/Colossus/commit/9e94ffe4d8fd5e6cd2c2f6398287ad5b77fde301))
- **browser:** apply terminal viewer revocation once ([02076cf](https://github.com/obscuritylabs/Colossus/commit/02076cfd6ef9ada866525b14f118a072f3b6390e))
- **browser:** retire viewers after explicit session cleanup ([41473f9](https://github.com/obscuritylabs/Colossus/commit/41473f92e690a439beb1efbce53d30eca35983a5))
- **browser:** preserve viewer recovery between sessions ([cd0a8d5](https://github.com/obscuritylabs/Colossus/commit/cd0a8d5dfa03233526a96aa2489ac03f29686138))
- **browser:** preserve visible views across superseded hides ([dbfeee0](https://github.com/obscuritylabs/Colossus/commit/dbfeee026554eebfffd910da7e119bd3fe759792))
- **grpc:** offload bounded artifact reads ([6a9c4f8](https://github.com/obscuritylabs/Colossus/commit/6a9c4f89b6bc5d40ac3689d0400cbefb142f9e45))
- **browser:** coalesce acknowledged hidden leases ([4df6676](https://github.com/obscuritylabs/Colossus/commit/4df6676ea8ff9960e5067d4f862bb015034e778f))
- resolve desktop testing regressions and add native logs ([dee9362](https://github.com/obscuritylabs/Colossus/commit/dee93622a2a8c1fe7518d3fb9112c859f3f514ec))
- propagate native navigation failures before observations ([0c1d39f](https://github.com/obscuritylabs/Colossus/commit/0c1d39f1e7c9ed314321cbdec924e12fabf01c83))
- reuse retained browser frame budgets on replacement ([a67388e](https://github.com/obscuritylabs/Colossus/commit/a67388e36c685c529cc5c6cc38c388ad6f807f37))
- bound tool context and add Windows client identity review ([40a8923](https://github.com/obscuritylabs/Colossus/commit/40a8923e5f1c0c28d2a2a3e8ddf637d0bd8552ec))

#### Security

- **dependencies:** replace vulnerable desktop and credential backends ([3cea322](https://github.com/obscuritylabs/Colossus/commit/3cea3224e398e78848c2315a039f44b1f0201a46))
- **dependencies:** replace vulnerable desktop and credential backends ([dd3525a](https://github.com/obscuritylabs/Colossus/commit/dd3525ab58d1a17a6f74fb3db35cafb4455fa861))
- patch bundled trust certificates and document native acceptance ([dfb02de](https://github.com/obscuritylabs/Colossus/commit/dfb02de134b6e488e5ea26b0e5e43ab596fe8c7f))
- patch frontend dev server and retain enrollment contract ([f8075a4](https://github.com/obscuritylabs/Colossus/commit/f8075a45e75e2c7043396a961c92a63c09fa0c6c))

#### Changed

- **storage:** seek Redb cursors and batch projection catch-up ([#286](https://github.com/obscuritylabs/Colossus/pull/286))

#### Documentation

- **release:** prepare 0.11.8 preview 2 history ([6994da7](https://github.com/obscuritylabs/Colossus/commit/6994da76d12820007d973a56f2d83a04f3f08d47))

#### Maintenance

- **deps-dev:** update desktop and web Vitest to 4.1.11 ([#292](https://github.com/obscuritylabs/Colossus/pull/292))
- **browser:** import expiry test clock for native builds ([afa8a99](https://github.com/obscuritylabs/Colossus/commit/afa8a99e60f689e84e6dbc77c3088c143fc9be72))
- **computer:** trace native foreground eligibility ([e84d921](https://github.com/obscuritylabs/Colossus/commit/e84d9218cc411be051ebbab1daa8fea432147cb6))
- **computer:** trace native input terminal receipts ([83e1f6f](https://github.com/obscuritylabs/Colossus/commit/83e1f6f67605ad0044bbfaca4ceb4aa3e82b42c8))
- **computer:** trace native control admission refusals ([8572c0c](https://github.com/obscuritylabs/Colossus/commit/8572c0c30e379bcbe9fbc9406ca8f19e1acb169b))
- **desktop:** verify authenticated computer tool discovery ([0bb3b9e](https://github.com/obscuritylabs/Colossus/commit/0bb3b9e0d6fd86e5ec6ff7806af24f1bf02afeb0))
- **desktop:** align provider inventory credential copy ([7561476](https://github.com/obscuritylabs/Colossus/commit/7561476968911908b75a90e5f7e501724dccf581))
- **release:** allow cold Intel macOS packaging to finish ([54bf5d6](https://github.com/obscuritylabs/Colossus/commit/54bf5d61ce2f9fd3807791bb5b3dd970201930cb))
- **ci:** validate bounded matrix timeout branches ([23761f9](https://github.com/obscuritylabs/Colossus/commit/23761f9a4d88a9c0924c29e740d4b7c99293cdd3))
- allow cold Windows Desktop acceptance to finish ([0299cac](https://github.com/obscuritylabs/Colossus/commit/0299cac53c5dd19c0322cda04eeb3e6034761060))
- refresh Windows release signing credentials ([024f166](https://github.com/obscuritylabs/Colossus/commit/024f16604f24de71bbf75c890a2e57d969e1127a))
- **browser:** report Windows SDK close evidence accurately ([9f1a395](https://github.com/obscuritylabs/Colossus/commit/9f1a3954e2c4d39af7b4200a1780866196341040))
- **browser:** trace hidden viewport polling failures ([0517afa](https://github.com/obscuritylabs/Colossus/commit/0517afa1bff8dca6d7f1f470fe42c83f78072be9))
- **browser:** reproduce modal visibility traffic ([d943cc1](https://github.com/obscuritylabs/Colossus/commit/d943cc1b6e891ba8719a9b50e6082e9741f78527))
- **browser:** distinguish wire and read delays ([f6cff19](https://github.com/obscuritylabs/Colossus/commit/f6cff197ce8428172eaae063f969801e5a0b08d0))
- **grpc:** trace slow history list polls ([ddbd3ef](https://github.com/obscuritylabs/Colossus/commit/ddbd3ef994847fe1b7e5ec0fdd52693c2386971f))
- **release:** prepare 0.11.9 changelog ([61d3d68](https://github.com/obscuritylabs/Colossus/commit/61d3d68747a2019edc16d67f621ac790a9b15576))
- assert live profile restoration and completed tool calls ([09a1d1d](https://github.com/obscuritylabs/Colossus/commit/09a1d1d26fb652df058577cda4e9c9e2528dd226))

**Source:** [09a1d1d](https://github.com/obscuritylabs/Colossus/commit/09a1d1d26fb652df058577cda4e9c9e2528dd226)

**Full changelog:** [v0.11.8...v0.11.9](https://github.com/obscuritylabs/Colossus/compare/v0.11.8...v0.11.9)

<!-- /colossus:release-notes -->

## [0.11.8-preview.2] - 2026-10-19

### Preview highlights

- Desktop browser views stay visible during agent actions and retain the completed
  page for inspection, with clear control indicators and inline screenshot previews.
- Browser settings and guidance, agent inbox navigation, secure provider setup, and
  model selection are improved.
- Opt-in Windows computer control adds observation-bound input for existing apps,
  visible Pause/Stop controls, and window screenshots. It requires an interactive user
  session and enabled workspace capability.
- Windows includes an explicitly labeled functional Chromium preview. Production
  browser containment and automatic Desktop updates remain unavailable.

<!-- colossus:release-notes base=v0.11.8 -->

### Merged changes

#### Added

- **http:** add bounded redirects and Desktop settings ([#289](https://github.com/obscuritylabs/Colossus/pull/289))
- **ui:** unify web workspace management and standalone Goal mode ([#287](https://github.com/obscuritylabs/Colossus/pull/287))
- **agents:** add durable inboxes and A2A application edge ([#288](https://github.com/obscuritylabs/Colossus/pull/288))
- **models:** switch models and reasoning in Desktop, VS Code and TUI ([#295](https://github.com/obscuritylabs/Colossus/pull/295))
- **provider:** add native Anthropic Messages support ([#297](https://github.com/obscuritylabs/Colossus/pull/297))
- **vscode:** bundle and manage local runtimes in the sidebar ([#296](https://github.com/obscuritylabs/Colossus/pull/296))
- **images:** add local image viewing and activity previews ([#298](https://github.com/obscuritylabs/Colossus/pull/298))
- **browser:** connect owned Chromium hosts to Core and Desktop ([#290](https://github.com/obscuritylabs/Colossus/pull/290))
- **browser:** add runtime settings and bundled browser guidance ([637e25c](https://github.com/obscuritylabs/Colossus/commit/637e25ce8e326fb1c9b8e25d0c8ea3f7bd00cd71))
- **computer:** add opt-in Windows desktop control ([c229164](https://github.com/obscuritylabs/Colossus/commit/c229164951faf8203c309dada2f1b6249935908b))
- **windows:** expose read-only process job accounting ([c3e4a41](https://github.com/obscuritylabs/Colossus/commit/c3e4a4194b3729a3554b3a83a5989443d2273692))
- **windows:** expose read-only process job accounting ([8c3d266](https://github.com/obscuritylabs/Colossus/commit/8c3d266f84c8967a29c800430f5e6985b795ef20))
- **browser:** package Windows preview and preserve agent views ([c8d82a8](https://github.com/obscuritylabs/Colossus/commit/c8d82a825f4a2dd29bcbf99b30e7604f8b23a97a))
- **browser:** package Windows preview and preserve agent views ([004ea37](https://github.com/obscuritylabs/Colossus/commit/004ea37e916574ebe19438d5777e0289dd87e959))

#### Fixed

- **plugins:** avoid OCI packaging for local snapshots ([#293](https://github.com/obscuritylabs/Colossus/pull/293))
- **desktop:** apply access settings without redundant prompts ([#294](https://github.com/obscuritylabs/Colossus/pull/294))
- **browser:** preserve input gestures and agent viewer continuity ([b6c57f2](https://github.com/obscuritylabs/Colossus/commit/b6c57f23ddb3438253066e65bac75e4be0f33dd1))
- **browser:** polish capability settings presentation ([492935a](https://github.com/obscuritylabs/Colossus/commit/492935a47ae59728e106512a82858f6b57bd3b8e))
- **browser:** retain focus after inheriting settings ([5b39d62](https://github.com/obscuritylabs/Colossus/commit/5b39d62ff8597d71ffd4d9b963cc0a6e248c5904))
- **browser:** load public pages and reveal agent tabs ([9989d72](https://github.com/obscuritylabs/Colossus/commit/9989d72d8e93150cd8b807b538c5c0d17f2508c0))
- **desktop:** preserve browser views and clarify agent settings ([88cc3d1](https://github.com/obscuritylabs/Colossus/commit/88cc3d1847d27e08972b005a5bf1a4a250239dbe))
- **desktop:** harden credential labels and native acceptance fixtures ([b3cc106](https://github.com/obscuritylabs/Colossus/commit/b3cc106fcce290adbc229e014117a772e3a2f584))
- **desktop:** upgrade Tao to prevent Windows keyboard deadlocks ([e9a2b80](https://github.com/obscuritylabs/Colossus/commit/e9a2b8061f0e82cde8cb9fccdc15767fedb424bc))
- **browser:** retain completed views and show agent control ([7111d76](https://github.com/obscuritylabs/Colossus/commit/7111d768d954309054ea7afef776571291d2f464))
- **browser:** retain acknowledged leases during viewport recovery ([54ab8f3](https://github.com/obscuritylabs/Colossus/commit/54ab8f3759a7d7c5094c7620a1c4e0afa7e0607f))
- **browser:** retire expired network sessions with clear recovery ([46444f4](https://github.com/obscuritylabs/Colossus/commit/46444f44c298c9bf575d747c69606e3516e68492))
- **codex:** update protocol and retain stream error diagnostics ([4e63bed](https://github.com/obscuritylabs/Colossus/commit/4e63bed240ec22a8ab1a3cb4f10b930cb48524c9))
- **computer:** retain verified Windows root for capture helpers ([6a5a730](https://github.com/obscuritylabs/Colossus/commit/6a5a7307fb05cc790ae41aa46521dfa040b05eb5))
- **desktop:** include computer tools in managed application grants ([1dab1a4](https://github.com/obscuritylabs/Colossus/commit/1dab1a4d316e4ccbf5fcf17911d9a7ca8a1de128))
- **tools:** accept attached search term prefixes ([3337a92](https://github.com/obscuritylabs/Colossus/commit/3337a924de5a408b6816837c1f726b7b28c5bc87))
- **computer:** settle foreground activation and preserve input markers ([fcd03c5](https://github.com/obscuritylabs/Colossus/commit/fcd03c5f67e4be4ed8c9da77d52f261055400b6b))
- **browser:** allow bounded slow metadata reads without disconnecting ([2a4c18a](https://github.com/obscuritylabs/Colossus/commit/2a4c18a1a1400410c7179a356df9892ab682261b))
- **browser:** allow bounded slow metadata reads without disconnecting ([055a7c8](https://github.com/obscuritylabs/Colossus/commit/055a7c815807307272ae011e3726e59342e45c3d))
- **desktop:** restore new-thread composer from inbox views ([2c824b4](https://github.com/obscuritylabs/Colossus/commit/2c824b49ce8c800d41c8ac6db4a2e4de6390f99a))
- **desktop:** restore new-thread composer from inbox views ([ca16256](https://github.com/obscuritylabs/Colossus/commit/ca162568c9c8b784b907c9590b526a6e45e817d9))

#### Security

- **dependencies:** replace vulnerable desktop and credential backends ([3cea322](https://github.com/obscuritylabs/Colossus/commit/3cea3224e398e78848c2315a039f44b1f0201a46))
- **dependencies:** replace vulnerable desktop and credential backends ([dd3525a](https://github.com/obscuritylabs/Colossus/commit/dd3525ab58d1a17a6f74fb3db35cafb4455fa861))

#### Changed

- **storage:** seek Redb cursors and batch projection catch-up ([#286](https://github.com/obscuritylabs/Colossus/pull/286))

#### Maintenance

- **deps-dev:** update desktop and web Vitest to 4.1.11 ([#292](https://github.com/obscuritylabs/Colossus/pull/292))
- **browser:** import expiry test clock for native builds ([afa8a99](https://github.com/obscuritylabs/Colossus/commit/afa8a99e60f689e84e6dbc77c3088c143fc9be72))
- **computer:** trace native foreground eligibility ([e84d921](https://github.com/obscuritylabs/Colossus/commit/e84d9218cc411be051ebbab1daa8fea432147cb6))
- **computer:** trace native input terminal receipts ([83e1f6f](https://github.com/obscuritylabs/Colossus/commit/83e1f6f67605ad0044bbfaca4ceb4aa3e82b42c8))
- **computer:** trace native control admission refusals ([8572c0c](https://github.com/obscuritylabs/Colossus/commit/8572c0c30e379bcbe9fbc9406ca8f19e1acb169b))
- **desktop:** verify authenticated computer tool discovery ([0bb3b9e](https://github.com/obscuritylabs/Colossus/commit/0bb3b9e0d6fd86e5ec6ff7806af24f1bf02afeb0))

**Source:** [8e750fd](https://github.com/obscuritylabs/Colossus/commit/8e750fd4294607486136eae953ce3dbab3b5a2f8)

**Full changelog:** [v0.11.8...v0.11.8-preview.2](https://github.com/obscuritylabs/Colossus/compare/v0.11.8...v0.11.8-preview.2)

<!-- /colossus:release-notes -->

## [0.11.8] - 2026-10-09

### Added

- Offline speech input in Desktop and TUI, and a VS Code extension using the shared
  Desktop UI and themes.
- Colossus Control Plane with independent PostgreSQL persistence, users, named OIDC
  and optional local sign-in, explicit project roles/hierarchy, and administration.
- Fleet agents, workspace/thread navigation, retained realtime conversations, shared
  Desktop/VS Code/web Markdown and composer components, and shadcn task tables.
- Project analytics, runtime-reported policy inspection and monitoring baselines,
  configurable classification markings, and global Desktop connection profiles.
- Linux Control Plane server/web bundles and offline Docker images, six platform
  VSIX packages, and verified multi-platform server container publication.

- Workspace workflow schedule management with definition import, reviewed creation,
  pause/enable controls, and independent run inspection.
- Separate Workflows library with manual runs, schema-based inputs, owned history, and
  bounded final results; Schedules supports plain-language tasks and reusable workflows.
- Daily and weekly schedules in IANA timezones, preserving local time across DST, plus
  configured task model profiles and reasoning effort through the normal provider path.
- Read-only workflow logic graphs with conditional branches, parallel joins, bounded
  loops, and recorded step states and completion counts.

- Six editable schedule examples with copyable agent prompts and the bundled
  `colossus/schedule-task` skill. Agents can use `workflow.task.schedule` through
  normal schedule permissions, review, ownership, and retry protections.

### Changed

- Aligned workspace, bundled plugin, and SDK candidate metadata to `0.11.8`.
  SDK registry publishing remains disabled for this release.
- Workflows and Schedules use shared UI typography and theme tokens, with a compact
  Providers-style schedule inventory and front-end development rules in `AGENTS.md`.

### Fixed

- Agent schedule creation and pause/enable approvals publish the valid public schedule
  review category, retaining exact response binding and owner isolation.
- Offline development credential rewrap on Windows fingerprints journals through
  their retained exclusive lease, preserving source bytes and writer exclusion.

<!-- colossus:release-notes base=v0.11.7 -->

### Merged changes

#### Added

- **vscode:** add Colossus extension with shared desktop UI and themes ([#263](https://github.com/obscuritylabs/Colossus/pull/263))
- **dictation:** add offline speech input to Desktop and TUI ([#264](https://github.com/obscuritylabs/Colossus/pull/264))
- **desktop:** add workflow schedules, agent tasks, and run output ([#262](https://github.com/obscuritylabs/Colossus/pull/262))
- **control-plane:** add runtime operations and shared workspaces ([#265](https://github.com/obscuritylabs/Colossus/pull/265))
- **research:** inherit enabled MCP tools and add VSIX controls ([#273](https://github.com/obscuritylabs/Colossus/pull/273))
- **mcp:** support 2026 Streamable HTTP with protocol selection ([#274](https://github.com/obscuritylabs/Colossus/pull/274))
- **ui:** streamline automations and fix dock sizing ([#276](https://github.com/obscuritylabs/Colossus/pull/276))
- **release:** generate changelog and release notes ([#277](https://github.com/obscuritylabs/Colossus/pull/277))
- **plugins:** deepen security review with bounded delegation and checkpoints ([#284](https://github.com/obscuritylabs/Colossus/pull/284))
- **plugins:** improve coding skill and remove offline-dev ([#283](https://github.com/obscuritylabs/Colossus/pull/283))
- **plugins:** load workspace sources and simplify adding plugins ([#281](https://github.com/obscuritylabs/Colossus/pull/281))

#### Fixed

- **tui:** shade the composer and use a single status row ([#260](https://github.com/obscuritylabs/Colossus/pull/260))
- **ci:** include schedule skill in Unix release smoke check ([#266](https://github.com/obscuritylabs/Colossus/pull/266))
- **release:** allow maintained documentation build helper ([#269](https://github.com/obscuritylabs/Colossus/pull/269))
- **desktop:** serialize run listings per target ([#270](https://github.com/obscuritylabs/Colossus/pull/270))
- **desktop:** surface context limits and provider rejection causes ([#272](https://github.com/obscuritylabs/Colossus/pull/272))
- **presentation:** identify MCP servers in tool calls ([#275](https://github.com/obscuritylabs/Colossus/pull/275))
- **release:** generate SDK bindings and fix acceptance races ([#271](https://github.com/obscuritylabs/Colossus/pull/271))
- **cli:** fingerprint journals through retained offline leases ([#279](https://github.com/obscuritylabs/Colossus/pull/279))
- **ci:** restore main macOS bundle cache for releases ([#282](https://github.com/obscuritylabs/Colossus/pull/282))

#### Changed

- **cloud:** normalize PostgreSQL storage with typed rows ([#280](https://github.com/obscuritylabs/Colossus/pull/280))

#### Maintenance

- **packaging:** pin published v0.11.7 archives ([#259](https://github.com/obscuritylabs/Colossus/pull/259))

**Source:** [b4399d2](https://github.com/obscuritylabs/Colossus/commit/b4399d213f10a86bbb3d05919d5ac2294da811af)

**Full changelog:** [v0.11.7...v0.11.8](https://github.com/obscuritylabs/Colossus/compare/v0.11.7...v0.11.8)

<!-- /colossus:release-notes -->

## [0.11.7] - 2026-10-03

### Added

- Automatic model capability discovery and Responses conversation compaction,
  with Auto/On/Off feature controls in Desktop.
- Full-screen TUI session, theme, and history browsers with a searchable list and
  a right-hand preview pane. Settings such as permissions open a current-value
  chooser while retaining direct command arguments.
- A persistent workspace/session top bar, readable highlighted status and warning
  badges, and a faint encoded Obscurity Labs mark on the welcome screen.

### Fixed

- Transient provider gateway failures retry with visible recovery status.
- Prompt history caches search matches between redraws, and preview paging
  stops at the end so PageUp responds immediately after repeated PageDown.
- Multiline composition accepts legacy newline input and exposes Ctrl-J as a
  newline shortcut. Cursor navigation follows visual rows in wrapped drafts.
- Welcome security warnings no longer hide the background mark, and narrow
  layouts preserve status, permissions, and warning counts without overlap.

### Changed

- TUI sessions use the alternate screen by default; `--no-alt-screen` preserves
  native scrollback, including resumed Herdr sessions.
- Aligned workspace, bundled plugin, and SDK candidate metadata to `0.11.7`.
  This release publishes Core/Desktop only; SDK registry publishing remains disabled.

## [0.11.6] - 2026-10-02

### Added

- Global PEM client certificate and private key configuration for Colossus-owned TLS
  requests, including provider, HTTP MCP, policy, database, and plugin registry calls.
- Desktop imports the identity into private native storage and supplies it to managed
  runtimes. CLI configuration accepts certificate and key paths.

### Fixed

- Desktop repository imports keep native client identity paths under global control.
- OPA decision requests reject redirects so client identity cannot follow a redirect.
- Release-build TUI command parsing retains its input behavior.

### Changed

- Desktop settings use compact catalogs and organized preferences, with updated guides.
- Aligned workspace, bundled plugin, and SDK candidate metadata to `0.11.6`.
  This release publishes Core/Desktop only; SDK registry publishing remains disabled.

## [0.11.5] - 2026-10-01

### Added

- Desktop Stop pauses queued messages for Resume, with a slow activity glow.
- Compact Shell and TUI controls, with a native system shell beside authenticated
  Colossus sessions. Active shells exposes managed background output and Stop.
- Tool access patterns such as `get_*` for built-in and MCP tool selections.
- Native command review offers Allow once and Always allow for the exact command
  and working directory in a workspace, with a control to clear remembered choices.
- Provider presentation settings include custom icons and Markdown instructions
  in portable Desktop setup packages.

### Fixed

- Managed shell commands retain interactive approval routing before detaching.
  Initial waits honor their yield budget, and denied or failed launches no longer
  appear successful. Terminal denials also finish their activity rows.
- Small supported output budgets retain stdout and stderr. Managed command previews
  include released output, status, exit codes, and truncation information.
- Active shells distinguishes loading, unavailable, and empty states and discards
  stale responses after workspace changes. Shell and TUI use the active native
  workspace, clearing prior workspace context during selection changes.
- Windows launches restore the original Desktop window instead of creating extra
  tray instances. Startup shows a loading state before choosing setup or Work.
- Credential export preserves readable names with empty import placeholders;
  rotation retains credential identity, and unreferenced revision history no longer
  prevents removal while active and pinned references remain protected.

### Changed

- Desktop-managed runtimes start in Risk Auto. Other SDK hosts retain Ask by default.
- Completion notifications include the thread title and a bounded output snippet.
- Aligned workspace, bundled plugin, and SDK candidate metadata to `0.11.5`.
  This release publishes Core/Desktop only; SDK registry publishing remains disabled.

## [0.11.4] - 2026-09-30

### Added

- A compact Tools menu and shared right pane for Files, Browser, Terminal,
  Changes, Artifacts, thread details, and Aside. Tool switching preserves
  conversation drafts, browser tabs, and local terminal sessions.
- The integrated browser is enabled in normal Windows Desktop builds, with
  isolated temporary sessions and native file-picker cancellation.

### Fixed

- Windows TUI windows now load their bundled document and assets correctly.
  ConPTY children receive console streams even when Desktop starts with
  redirected standard handles.
- The local TUI opens beside the conversation with bounded native positioning,
  workspace-scoped cleanup, and terminal-only PTY permissions.

### Changed

- Aligned workspace, bundled plugin, and SDK candidate metadata to `0.11.4`.


## [0.11.3] - 2026-09-29

### Added

- Desktop setup packages can share global defaults, sandbox settings, limits, MCP
  servers, search providers, and telemetry profiles. Export the current global
  configuration with portable credential placeholders for offline onboarding.
- Desktop model routing displays all seven roles with their responsibilities,
  model-selection guidance, and the effective primary fallback.
- Added macOS menu-bar and Windows system-tray controls for the Desktop lifecycle.

### Changed

- Model discovery recognizes additional context-window and capability fields from
  compatible providers, including vLLM and Azure-style responses, while preserving
  explicit unsupported capabilities.
- Provider and model connection tests now appear beside their settings rows, with
  progress, inline results, detailed failures, and retry.
- Aligned workspace, bundled plugin, and SDK package metadata to `0.11.3`.

### Fixed

- MCP Test and OAuth actions remain unavailable until workspace selections are
  applied and the runtime is active. Pending changes no longer appear enabled.
- Imported setup providers remain available throughout onboarding, with clearer
  workspace activation and optional credential enrollment.
- Windows uninstall cleanup accepts an empty generated CLI home without removing
  unrecognized or shared application data.

## [0.11.2] - 2026-09-28

### Added

- Added offline Desktop setup packages with canonical YAML provider/model settings,
  Markdown onboarding instructions, bundled provider icons, and optional CA certificates.
  Imports preview endpoints, models and trust changes before saving; credentials stay
  in the native vault and can be added when the provider is first used.
- Added a five-provider sample package and a documented manifest schema for administrators.

### Changed

- Kept Desktop import compact and moved provider instructions, credential setup and
  model selection into their respective onboarding steps. Imported model limits,
  capabilities, reasoning settings and provider timeouts survive workspace activation.
- Updated the coordinated workspace, bundled plugin and SDK metadata to `0.11.2`.

### Fixed

- Windows Desktop application-data cleanup now accepts hard links contained entirely
  within the plugin cache and explains why cleanup stops instead of always reporting
  running tasks. External hard links and unrecognized data remain protected.

## [0.11.1] - 2026-09-28

### Fixed

- Fixed Desktop model loading and installation checks after deleting and recreating
  the application data folder. Setup now binds its protected journal to the actual
  directory identity and automatically uses a new diagnostic namespace for older
  installations, preserving workspace conversations and their integrity checks.
- Fixed Windows uninstall's optional application-data cleanup to include the default
  Desktop home and its identifiable protected keys. A destructive-data confirmation
  explains what is lost; normal uninstall and updates continue to preserve data.
- Removed misleading provider URL and sign-in advice from local runtime failures
  displayed during model discovery.

## [0.11.0] - 2026-09-27

### Added

- Added Azure Artifact Signing for Windows release executables and Desktop installers,
  with signed CLI and sidecar binaries inside the Windows Desktop bundle.
- Added managed ripgrep to CLI and Desktop release bundles so search works without a
  separate host installation.
- Added Desktop browser preview, streamlined setup and settings, provider icons, and
  model-aware composition.
- Added bounded MCP tool discovery for agents and workspace-wide decisions.

### Changed

- Improved provider discovery across CLI and Desktop and made Shift+Enter insert a
  newline in the terminal interface.
- Released the Rust workspace, bundled plugin, Desktop fixtures, and TypeScript,
  Python, and Go SDKs as `0.11.0`.

### Fixed

- Reported the host operating system to shell execution and allowed checked pull
  requests to merge after `main` advances when no merge conflict exists.

## [0.10.10-preview.19] - 2026-09-24

### Added

- Added an encrypted credential vault for Windows, macOS, and Linux. Native OS
  credential stores protect small vault keys while large API tokens and OAuth
  records remain encrypted in private application storage.
- Added native masked credential entry on Windows and macOS with exact byte counts,
  long-token paste support, keyboard controls, and Desktop appearance settings.

### Fixed

- Preserved opaque API and MCP tokens up to 64 KiB through Desktop persistence,
  managed-runtime startup, provider requests, and authenticated MCP discovery and
  tool calls. Serialized OAuth credential records support up to 1 MiB.
- Kept OAuth credentials usable when the state database is renamed within its
  private directory.

### Changed

- Replaced the legacy Windows credential prompt and direct full-token OS storage.
  Existing saved API tokens must be re-entered and platform-stored OAuth
  connections reauthorized; connection references remain available for
  reconnection, and previous OS credential entries are left untouched.
- Synchronized Rust package versions, exact internal dependency pins, lockfiles,
  the embedded core plugin, and Desktop fixtures to `0.10.10-preview.19`.

## [0.10.10-preview.18] - 2026-09-24

### Added

- Added `colossus mcp doctor SERVER` and Desktop connection diagnostics with failure
  stages, sanitized error categories, runtime versions, and endpoint and CA fingerprints.
- Included recent MCP health reports in Desktop diagnostic exports while excluding
  credentials, endpoint text, certificate paths, headers, and response bodies.

### Changed

- Unified Desktop Settings in a dedicated view with consistent Global and Workspace
  navigation, workspace selection, contextual help, and a Back to work action.
- Made top-of-app security banners opt-in through the persistent Show security
  warnings appearance setting, without changing runtime protections.
- Improved MCP diagnostic layout, tool discovery counts, and export access.
- Synchronized Rust package versions, exact internal dependency pins, lockfiles, the
  embedded core plugin, and Desktop fixtures to `0.10.10-preview.18`.

## [0.10.10-preview.17] - 2026-09-23

### Fixed

- Preserved MCP tool schemas and host or environment credential-header references
  through policy preparation so authenticated servers can discover and call tools.
- Kept argument secrets redacted and rejected secret-bearing schema literals,
  including values reached through local schema references.

### Changed

- Synchronized Rust package versions, exact internal dependency pins, lockfiles, the
  embedded core plugin, and the Desktop fixture to `0.10.10-preview.17`.

## [0.10.10-preview.16] - 2026-09-21

### Fixed

- Canonicalized the Windows Desktop worker endpoint before deriving its named pipe,
  keeping MCP diagnostics connected to the managed sidecar.
- Kept Windows plugin cleanup and corruption fixtures compatible with strict Clippy
  checks while retaining explicit private permission modes on Unix.

### Added

- Windows stdio MCP acceptance in pre-merge CI and opt-in public Cloudflare Docs
  discovery and tool-call checks for the CLI and native sidecar.
- Desktop instructions for testing a stateless public HTTP MCP server.

### Changed

- Added saved MCP server deletion and widened the Desktop chat layout.
- Synchronized Rust package versions, exact internal dependency pins, lockfiles, the
  embedded core plugin, and the Desktop fixture to `0.10.10-preview.16`.

## [0.10.10-preview.15] - 2026-09-17

### Changed

- Spelled out `Obscurity Labs // COLOSSUS` in the persistent terminal footer.
- Synchronized Rust package versions, exact internal dependency pins, lockfiles, the
  embedded core plugin, and the Desktop fixture to `0.10.10-preview.15`.

### Fixed

- Kept the full Obscurity Labs name and security warning count visible when the
  terminal is too narrow for the complete footer branding.

## [0.10.10-preview.14] - 2026-09-16

### Fixed

- Kept Plan mode focused on producing one draft: planning cannot create tasks, and
  every planned step must explicitly declare whether its eventual execution mutates
  state.
- Preserved live clarification and planning after a user answers a question by
  delivering the tool-start event before opening the interaction.
- Retired Desktop plan controls as soon as a revision or execution is accepted,
  including queued and waiting runs, preventing duplicate continuation requests.
  Preserved the planning response through execution and returned the composer to
  Execute mode after direct or Goal continuation.
- Increased the macOS pre-merge Desktop job limit to accommodate browser and native
  acceptance followed by cold optimized CLI, sidecar, and app builds.

### Security

- Updated Rustls to 0.23.45 in the core and Desktop dependency graphs to address
  RUSTSEC-2026-0285, with the required WebPKI and AWS-LC dependency updates.

### Changed

- Synchronized Rust package versions, exact internal dependency pins, lockfiles, the
  embedded core plugin, and the Desktop fixture to `0.10.10-preview.14`.

## [0.10.10-preview.13] - 2026-09-11

### Added

- Required a concise, task-specific justification for new model-issued `shell.run`
  commands and displayed it separately from policy and risk information in CLI, TUI,
  Desktop, and SDK approval interactions.
- Added full command review with prepared executable, argument boundaries, working
  directory, display-only credential redaction, and immutable approval binding. Desktop
  review uses an isolated native-owned window and retains native final confirmation.
- Added data-only OCI release images containing every published release asset, with
  authenticated Docker Hub publication and GHCR fallback, verified inventories,
  checksum validation, and digest-pinned offline transfer.

### Changed

- Bumped the authenticated worker protocol to 22 and added optional public command
  context without changing schema-version-3 configuration or historical approvals.
- Synchronized Rust package versions, exact internal dependency pins, lockfiles, the
  embedded core plugin, and the Desktop fixture to `0.10.10-preview.13`.

### Fixed

- Shared authenticated sidecar approval-broker routing across macOS and Windows.
- Prevented raw shell invocation details from bypassing the sanitized approval context
  through live or restored public tool activity.
- Preserved post-operation Desktop plugin inventory refreshes and ordered recent
  sessions by parsed timestamps rather than RFC3339 string formatting.

## [0.10.10-preview.12] - 2026-09-08

### Fixed

- Created Windows release smoke fixtures beneath the runner's user profile with explicit
  owner-private ACLs and isolated plugin homes. Added real-runtime fixture acceptance to
  the Windows pre-merge gate so unsafe shared temporary roots fail before tagging.
- Preserved sticky-directory bits when inspecting macOS installer ancestry, allowing
  safe private installations beneath sticky temporary directories while continuing to
  reject replaceable non-sticky ancestors.

### Changed

- Superseded the unpublished preview 11 candidate after its Windows release-fixture
  validation failed. This preview includes the embedded core plugins, OCI distribution,
  Desktop/TUI plugin experience, and schema-v3 hard cutover described below.
- Synchronized Rust package versions, exact dependency pins, lockfiles, the embedded core
  plugin, and the Desktop fixture to `0.10.10-preview.12`.

## [0.10.10-preview.11] - 2026-09-08

Unpublished candidate; superseded by preview 12.

### Added

- Embedded the `colossus` core plugin in the CLI and sidecar with `coding`,
  `offline-dev`, `security-review`, and `plugin-authoring` skills, available on first
  launch without a checkout, registry connection, or manual installation.
- Added home-wide plugin lifecycle management, deterministic OCI packaging and offline
  transfer, configured registry authentication and trust, and explicit enablement of
  plugin-provided MCP servers.
- Added Desktop and TUI plugin management, instruction and resource previews, qualified
  per-message skill mentions, and visible conversation-level skill selections.
- Added bounded plugin icons and improved Desktop plugin discovery, with matching
  public API and SDK display metadata.

### Changed

- Replaced the custom skills, packs, and collections system with Agent Plugins and
  Agent Skills. Configuration schema version 3 is a hard cutover: regenerate older
  extension configuration; legacy extension data is not migrated.
- Kept run catalogs immutable and leased across execution, with workspace policy
  narrowing the globally active plugin set and normal tool authority applying to
  selected skill resources and scripts.
- Bumped the Rust workspace, exact internal dependency pins, Desktop native and fuzz
  dependencies, lockfiles, and bundled core plugin to `0.10.10-preview.11`.

### Fixed

- Restored Plugins navigation in the production Desktop sidebar and added keyboard,
  compact-layout, native command-adapter, worker, and TUI acceptance coverage.
- Enforced credential-file, credential-helper, and trust-root authority before plugin
  operations; preserved authorized disabled-plugin discovery and exact skill identities.
- Bounded local and external plugin icon loading and encoded response sizes, including
  the complete data URL, and paginated discovery by its encoded size.
- Added a version-5 filesystem-scoped opaque-handle identity for Linux NFS workspaces
  that do not report birthtime, while preserving the existing birthtime identity and
  failing closed on unsupported, malformed, or inconsistent identity evidence.
- Documented that NFS workspace binding does not guarantee state-backend semantics on
  an NFS user home and recommended an owner-private local `COLOSSUS_HOME` for local
  workspace state.

## [0.10.10-preview.10] - 2026-09-02

### Added

- Added a keyboard-accessible Desktop slash-command menu for core modes, Plan and
  Research controls, task navigation, permission modes, product destinations, and the
  authenticated local TUI. Commands execute locally and are never submitted as model
  prompts.
- Added shared Desktop toast notifications for successful settings operations and local
  command feedback.
- Added subtle Obscurity Labs identity to the persistent Desktop sidebar and the TUI Lab
  Console, including theme-aware use of the canonical OL mark and brand red.

### Changed

- Replaced oversized Desktop model and MCP checkboxes with compact switches, moved
  settings feedback into transient notifications, and kept the Workspace apply bar at
  the bottom of the settings layout without leaving unused page space.
- Rendered context-snapshot summaries as bounded Markdown previews with a dedicated
  scrollable details pane.
- Removed the redundant global Activity destination while retaining task-scoped activity
  in the canonical conversation workspace.
- Expanded the TUI completion surface on roomy terminals and refined the responsive Lab
  Console, runtime status rows, composer, and persistent footer.
- Bumped the Rust workspace, internal dependency pins, Desktop native dependencies, fuzz
  dependency, and all corresponding lockfiles to `0.10.10-preview.10`.

### Fixed

- Accepted a sole `*` MCP tool allowlist through the Desktop managed protocol, preserved
  canonical provider and model catalog pins during reinitialization, and reported
  generated runtime configuration failures without misclassifying them as enrollment
  failures.
- Opened full-access Managed Local runtimes with loopback wildcard MCP servers while
  retaining exact network destinations and bounded managed configuration.
- Prevented deeply composed debug-sidecar effect futures from exhausting Tokio worker
  stacks, and moved disclosure-summary hashing onto a bounded blocking worker without
  changing the audit contract.
- Kept thread action menus stable through focus transitions so Archive can complete, and
  kept context-snapshot details independently scrollable at compact window sizes.

## [0.10.10-preview.9] - 2026-09-01

### Added

- Added first-class delegated-agent lifecycle updates to parent run streams and terminal
  presentation, including queued, running, completed, failed, cancelled, and interrupted
  states with bounded task and result details.
- Added an independent `generationTimeoutMs` hard ceiling for streaming provider calls
  while retaining `timeoutMs` as the resettable connection and read-inactivity limit.

### Changed

- Refilled delegated-agent concurrency slots as child runs finish so queued work no longer
  waits for an entire scheduler batch to settle.
- Bumped the Rust workspace, internal dependency pins, Desktop native dependencies, fuzz
  dependency, and all corresponding lockfiles to `0.10.10-preview.9`.

### Fixed

- Allowed healthy provider streams to continue while bytes are arriving, while still
  bounding stalled streams and the total generation duration with separate deadlines.
- Preserved Markdown structure, spacing, line endings, and Unicode in terminal pastes,
  with retained placeholders for large submissions instead of flattening their content.
- Kept runtime-authored delegated-agent lifecycle metadata distinct from child-controlled
  output while preserving bounded, parseable progress and result previews.
- Cancelled parent runs when their public event observer fails while still settling delegated
  children, and flushed already-accepted provider text before hard-deadline errors.
- Preserved the complete configured provider generation budget while retaining a separate
  cleanup allowance, and published delegated-agent cancellation at its durable transition.
- Cleared transient inline composer and footer rows during terminal shutdown without
  removing committed transcript output.
- Made the first `Ctrl-C` clear a non-empty terminal draft; a subsequent press with an
  empty composer cancels an active run or exits when idle.

## [0.10.10-preview.8] - 2026-08-30

### Changed

- Bumped the Rust workspace, internal dependency pins, Desktop native dependencies, fuzz
  dependency, and all corresponding lockfiles to `0.10.10-preview.8`.

### Fixed

- Applied the 256 KiB aggregate provider-visible tool-observation budget across the
  complete user logical turn, including sequential assistant/tool cycles, so many
  individually bounded MCP results cannot overflow the protected context tail.
- Preserved the original byte count and SHA-256 digest when an already-bounded tool
  observation is projected again under a smaller logical-turn allocation, while fresh
  tool output always derives provenance from its actual released bytes.
- Coalesced consecutive streamed model-text deltas into ordered batches of at most
  4 KiB or 100 ms before post-effect release, preventing provider tokenization from
  amplifying one long response into tens of thousands of policy and journal events;
  accepted buffered text is released before a later stream failure is returned.
- Resolved effect-stream concurrency from the indexed journal tail instead of replaying
  the complete effect stream before every streamed release event.

## [0.10.10-preview.7] - 2026-08-29

### Added

- Added system, light, and dark Desktop appearance preferences with a shared theme
  provider, accessible settings controls, and theme-aware syntax highlighting.
- Added bounded provider-visible tool observations that preserve structured salient
  fields and head/tail previews while omitting encoded binary data and recording the
  original byte count plus SHA-256 digest.

### Changed

- Split the runtime composition surface into focused access-policy, adapter, storage,
  goal, and plan modules while preserving its public behavior and thin crate root.
- Expanded Desktop managed settings, Workspace resource inspection, and browser
  acceptance coverage for the complete supported configuration surface.
- Made background projection maintenance drain bounded follow-up work promptly so long
  agent and workflow runs do not leave avoidable shutdown work behind.
- Bumped the Rust workspace, internal dependency pins, Desktop native dependencies, fuzz
  dependency, and all corresponding lockfiles to `0.10.10-preview.7`.

### Fixed

- Normalized complete MCP JSON-RPC errors into confirmed, redacted tool failures while
  preserving outcome-unknown handling for transport loss, malformed responses, and
  post-dispatch timeouts.
- Returned model-selected unknown MCP servers, unadvertised tools, and schema-invalid
  arguments as recoverable observations with bounded tool-name guidance; explicit
  configured allowlist denials remain terminal and fail closed.
- Kept complete released tool results in canonical run evidence while projecting bounded
  valid JSON into session history, including when resuming legacy oversized histories.
- Rejected oversized or non-printable provider tool-call identifiers before any tool can
  execute, preserving the model-observation byte ceiling and transcript correlation.
- Kept appearance changes synchronized across open Desktop windows and made denied
  renderer storage access fall back safely during startup and preference updates.
- Kept Desktop destination names stable when visual attention counts are present and
  kept every canonical Session Map control in view at the supported compact viewport.
- Restored keyboard access and WCAG AA line-number contrast for scrollable Desktop
  artifact previews in both light and dark themes.
- Corrected workspace-file navigation semantics, scroll-region keyboard access, and
  explorer path contrast.
- Bounded provider-controlled tool names in reduced model-observation metadata.

## [0.10.10-preview.6] - 2026-08-28

### Added

- Added bounded context-snapshot inspection to the Desktop session map, including a
  dedicated list, complete snapshot details, topology nodes, and live refresh while the
  Snapshots or Resources view is active.
- Added list-and-inspect access from Desktop Resources for every durable session-map
  family: delegated agents, goals, tasks, plans, decisions, memories, context snapshots,
  research runs, sources, and released artifacts.

### Changed

- Expanded Desktop catalog editors to preserve the full managed provider, model,
  telemetry, and MCP contracts. MCP revisions now expose exact line-delimited arguments
  and tool names, working directories, multiple environment and header credential
  bindings, non-secret static headers, OAuth, research projections, stateless HTTP, and
  nullable runtime limits.
- Added field descriptions to every managed setting and explicit controls for provider
  default timeouts, model reasoning effort, image-input capability, and sensitive
  telemetry acknowledgment.
- Bumped the Rust workspace, internal dependency pins, Desktop native dependencies, fuzz
  dependency, and all corresponding lockfiles to `0.10.10-preview.6` in the release PR.

### Fixed

- Made the Unix direct installer compatible with Ubuntu's common user-private-group
  `0002` umask. Missing installation directories are now created privately, and an
  existing group-writable `bin` directory is normalized only beneath an owner-private
  prefix; replaceable prefixes, group-writable bins under accessible prefixes, and
  world-writable destinations still fail closed with explicit installation-failure
  output.
- Kept Desktop settings apply failures visible in the sticky action bar instead of
  clearing the only actionable error during a managed-runtime restart.
- Released SDK-owned run and artifact channels before sidecar shutdown so applying a
  managed configuration can restart cleanly instead of timing out and rolling back.
- Prevented edits from silently replacing nullable provider timeouts, model reasoning
  effort, telemetry acknowledgments, or advanced MCP fields with UI defaults.

## [0.10.10-preview.5] - 2026-08-25

### Added

- Added a canonical, source-filterable session activity projection and Desktop trajectory
  view for inspecting run, model, tool, policy, usage, and subagent events.
- Added a live React Flow session topology that uses the full available pane and refreshes
  while the topology tab is active.
- Added persistent, Workspace-scoped thread names to the Desktop thread actions menu.

### Changed

- Renamed the Desktop's user-facing `Space` terminology to `Workspace` while retaining the
  existing persisted settings and native command schema for compatibility.
- Replaced native Desktop selects with accessible, app-owned dropdowns that have consistent
  sizing, keyboard navigation, and styling across platforms.

### Fixed

- Kept stdio MCP input open through the final JSON-RPC response and bounded completion-line
  scanning so local MCP servers can finish requests without premature EOF or unbounded reads.
- Prevented delayed Workspace drawer initialization from stealing focus after a user changes
  the thread-search scope.
- Centered Desktop run-timeline markers and normalized toggle sizing in settings and the
  session map.
- Prevented stale paginated activity responses from being merged after a session, search,
  or filter change.
- Preserved required-field validation and unavailable-value disclosure in the app-owned
  dropdown control.
- Added a bounded connection-refused retry to native-sandbox release acceptance so a
  transient authenticated-proxy readiness race cannot fail an otherwise valid artifact.

### Upgrade Notes

- `v0.10.10-preview.5` supersedes the unpublished `v0.10.10-preview.4` attempt. Its
  ARM64 macOS release job twice received a transient connection-refused result from the
  authenticated loopback proxy; the release gate prevented draft creation both times.
- Preview upgrades are manual. Download later preview installers and their checksum
  sidecars from GitHub Releases.

## [0.10.10-preview.4] - 2026-08-25

### Added

- Added a canonical, source-filterable session activity projection and Desktop trajectory
  view for inspecting run, model, tool, policy, usage, and subagent events.
- Added a live React Flow session topology that uses the full available pane and refreshes
  while the topology tab is active.
- Added persistent, Workspace-scoped thread names to the Desktop thread actions menu.

### Changed

- Renamed the Desktop's user-facing `Space` terminology to `Workspace` while retaining the
  existing persisted settings and native command schema for compatibility.
- Replaced native Desktop selects with accessible, app-owned dropdowns that have consistent
  sizing, keyboard navigation, and styling across platforms.

### Fixed

- Kept stdio MCP input open through the final JSON-RPC response and bounded completion-line
  scanning so local MCP servers can finish requests without premature EOF or unbounded reads.
- Prevented delayed Workspace drawer initialization from stealing focus after a user changes
  the thread-search scope.
- Centered Desktop run-timeline markers and normalized toggle sizing in settings and the
  session map.
- Prevented stale paginated activity responses from being merged after a session, search,
  or filter change.
- Preserved required-field validation and unavailable-value disclosure in the app-owned
  dropdown control.

## [0.10.10-preview.3] - 2026-08-21

### Added

- Added a revisioned global and Workspace managed-configuration control plane with
  effective-value provenance, validation, immutable resource revisions, repository
  import, diagnostics, and runtime lifecycle handling.
- Added enhanced Desktop settings for providers, models, credentials, MCP, telemetry,
  defaults, runtime, access, sandbox, search, research, advanced configuration,
  authority review, and sanitized effective YAML.

### Changed

- Extended the unsigned Developer Preview with the enhanced Windows Desktop settings
  experience and its release-tested Windows x64 installer and runtime path.
- Raised the default sandbox output ceiling from 1 MiB to 4 MiB and the process-tree
  memory ceiling from 256 MiB to 1 GiB while preserving explicit deployment overrides;
  the bounded post-effect policy input and worker protocol v15 response frame capacities
  also rise so permitted large results survive policy inspection and authenticated IPC
  encoding.
- Moved the optional SearXNG development stack from the repository root to
  `examples/services/` and replaced the stale internal reconstruction archive with a
  current feature inventory, Rust practices, test strategy, and architecture decisions.

### Removed

- Removed the deprecated nested `research.search` adapter; configure named top-level
  `search.profiles` and `search.roles.research` instead.
- Removed unversioned Python-era custom-theme imports and unused `repl_history` runtime
  aliases. Custom themes now require the documented schema-version-1 format.

### Fixed

- Hardened Windows private application storage, native credential handling, sidecar
  startup, version-bound packaging, and installed-app launch behavior.
- Preserved managed telemetry installation and historical credential revisions across
  sidecar startup and settings changes.

## [0.10.10-preview.2] - 2026-08-18

### Changed

- Committed Desktop activity to one timeline view and replaced generic tool and research
  event names with concise, task-oriented labels.
- Refined the Workspace sidebar with a trailing new-thread action, persistent local pinning
  during startup, and one contextual control for loading older threads.
- Reworked Research settings into a compact, accessible popover with persistent depth
  and evidence-source selections.

### Fixed

- Rebound explicitly reselected folders to their existing Workspace and repaired stale
  same-path duplicates after a folder object is replaced.
- Made Workspace archive and restore visible and functional in the Desktop development
  fixture, selected the next active Workspace after archiving the current one, and surfaced
  native lifecycle failures beside the Workspace controls.
- Retried bounded transient run-list admission failures during Desktop startup and
  background Workspace inspection instead of exposing a spurious empty or failed state.
- Kept thread action menus usable across WebKit focus transitions and preserved pin
  changes across reloads.

## [0.10.10-preview.1] - 2026-08-16

### Added

- Added a canonical Desktop session topology that visualizes delegated agents, goals,
  tasks, plans, key decisions, memories, research, sources, and artifacts in a dedicated
  tab with bounded, workspace-scoped inspection.
- Added a Slack-style multi-workspace navigation surface, thread pinning and archiving,
  background attention routing, side conversations, queued and redirected follow-ups,
  research mode, source inspection, plan review, and thread/session details.

### Changed

- Refined Desktop timelines, tool previews, composer behavior, resizable panes, and
  message controls for denser activity review without losing canonical context.
- Extended the unsigned Desktop Developer Preview release path with a smoke-tested
  Windows x64 NSIS installer, checksum, sealed bundle manifest, and provenance record.

## [0.10.9] - 2026-08-13

### Added

- Added `storage.adapter: ephemeral` for one-shot process-local runs. It uses redb's
  in-memory backend for the canonical journal and projections, keeps the default
  Tantivy and automatic MCP OAuth state in memory, creates no canonical state or
  writer-lock file, and reports the loss of restart/recovery evidence explicitly.
  Because the journal is process-local, `colossus worker` modes that serve or attach to
  another process are rejected under this adapter; `worker --once` still drains the
  running process.
- Added a responsive TUI launch rail for empty sessions with workspace, branch, model,
  execution-boundary, approval, durability, and security-posture context.
- Added repeated submitted-input history traversal in terminal sessions.

### Changed

- Fresh inline TUI sessions now clear the visible terminal viewport while retaining
  earlier shell rows in native scrollback.
- Security-posture guidance is now compact, visually separated from the composer, and
  paired with a higher-contrast persistent status rail.
- Repository file summaries now enforce a 64 KiB serialized result ceiling in addition
  to their line bound, including byte-safe truncation for previews and extracted lists.

### Fixed

- Collapsed finalized streaming output into native scrollback without leaving a large
  blank inline viewport behind.
- Prevented an oversized recent `repo.file_summary` result from making automatic context
  compaction fail even after older history was summarized.

## [0.10.8] - 2026-08-13

### Added

- Added recursively sparse schema-version-2 configuration: only `schemaVersion` and
  `storage.path` are required, while omitted defaultable groups and fields resolve to
  inspectable compiled defaults.
- Added an explicit Managed Local and embedded-SDK execution-boundary choice so hosts
  can select full access, workspace isolation, or offline isolation independently from
  approval mode.
- Added the ChatGPT subscription-backed Codex provider to Desktop Managed Local,
  including native official-CLI sign-in/sign-out status and per-model reasoning effort.
- Added the shared owner-private Colossus home, opaque per-workspace CLI/Desktop state
  partitions, `COLOSSUS_HOME`, and `storage.location: home_workspace`.
- Added automatic bounded home and repository `AGENTS.md` instruction snapshots for
  top-level user runs, Goal iterations, and delegated subagents.

### Changed

- Generated configuration now omits default agent and subagent runtime-limit blocks,
  while `config show` reports the resolved turn and concurrency bounds that remain
  enforced at runtime.
- Sparse schema-version-2 configurations now default to `access.profile: allow_all`
  and acknowledged `sandbox.backend: danger_full_access`. This intentionally unsafe
  pre-1.0 default applies across CLI, TUI, Desktop Managed Local, embedded SDK hosts,
  workflows, and background effects unless an isolating execution boundary is selected.
- `config init` now writes only `schemaVersion` and the selected storage location/path
  for the ordinary global or local case. `config show` continues to render the complete
  resolved configuration.
- Under acknowledged danger full access, structured filesystem and HTTP adapters use
  request-bound ambient authority as well as child processes. Absolute host paths and
  every canonical HTTP(S) origin—including loopback, private, link-local, and metadata
  origins—are in scope. Configured provider routes, credentials, MCP/integration
  declarations, extension trust, permits, audit, quarantine, transport validation, and
  configured bounds remain mandatory. Direct Unix timeout and output bounds cover the
  supervised effect, while process-count, memory, and cleanup are best-effort for
  deliberately detached descendants; strict containment requires native, OCI, Windows
  Job, or a containing external host boundary.
- Enabled pack tools and pack-declared stdio MCP servers are rejected under
  `danger_full_access`, where direct ambient execution cannot enforce their manifest
  resource and credential ceilings.
- `-w, --workspace` now denotes repository context, relative-path anchoring, and state
  partition identity rather than a universal filesystem security boundary when full
  access is active.
- `config init` now creates the user-level configuration, while `config init --local`
  creates a complete repository replacement. Resolution is explicit, repository, then
  home without merging.
- Desktop Managed Local now starts fresh under the workspace's Desktop home partition;
  earlier application-support data is preserved but ignored.
- Desktop Managed Local's **Offline isolated** boundary keeps the selected provider's
  exact service and authentication/refresh destinations while hiding the generic
  model-visible `network.http`, `web.fetch`, and `docs.fetch` tools. It is not an air-gap
  declaration.
- Fresh Desktop schema-v4 settings default to **Allow all** and **Full access**. Migrations
  from schema v1–v3 preserve the earlier effective isolation: Minimal maps to Offline
  isolated, while Development and legacy `allow_all` map to Workspace isolated.
- Codex CLI account commands now report completion only after Colossus verifies their
  runtime credential postcondition; unsafe stores are rejected before the official CLI
  is invoked, and logout must leave no usable credential behind.

### Fixed

- Kept TUI history and preference persistence inside the accepted per-session
  direct-execution boundary, avoiding a misleading non-fatal persistence error after
  the operator acknowledges full access.

### Security

- Kept Codex tokens and account identifiers out of the Desktop WebView, settings, and
  generated runtime YAML; the native host passes only the validated owner-private auth
  file path over inherited sidecar bootstrap IPC.
- Open Codex auth files with no-follow, nonblocking Unix semantics so special files such
  as FIFOs fail promptly instead of stalling credential readiness checks.
- Required no-follow owner-private Colossus homes and confined home-workspace storage;
  direct installers create only the empty root, and privileged Unix installs defer it
  until an end-user launch.
- Kept `AGENTS.md` outside authorization: it cannot widen tools, policy, approvals,
  sandbox roots, network origins, or immutable runtime instructions.

## [0.10.7] - 2026-08-11

### Fixed

- Accepted GitHub's compact versioned-API JSON in the Unix bootstrap installer while
  retaining bounded metadata, release identity, channel, and asset-name validation.

## [0.10.6] - 2026-08-10

### Added

- Added install-aware `colossus update` replacement for validated direct installations,
  with exact-version selection, downgrade refusal, the reviewed fixed-origin bootstrap
  embedded in the binary, and a detached Windows handoff for locked executables.
- Added unauthenticated macOS, Linux, and Windows installation verification after every
  published stable release.
- Added a locked four-platform Nix flake plus a reviewed two-architecture Homebrew
  formula and checksum-driven formula generator. Package-manager wrappers advertise
  upgrade guidance without gaining direct self-update authority.

### Changed

- Enabled npm provenance for trusted SDK publication now that the source repository is
  public.

### Security

- Disabled ambient proxy use explicitly in the Unix bootstrap metadata and asset
  downloads while retaining exact HTTPS origin and redirect checks.
- Updated Ratatui's cache dependency to patched `lru` 0.18.2 and documented a narrow,
  reachability-reviewed `RUSTSEC-2026-0253` exception for Tantivy 0.26.1 until its
  already-merged `lru` 0.18.2 update is published.

## [0.10.5] - 2026-08-08

### Added

- Added explicitly acknowledged direct-execution sandbox modes, including a dangerous
  full-access profile that can use ambient executables, permitted process resources,
  host working directories, and child-process networking after acknowledgement.
- Added keyless plaintext redb and PostgreSQL storage for deployments that intentionally
  rely on host storage controls, with bounded startup and worker compatibility checks.
- Added dense, searchable session and theme browsers, generated command help, and an
  explicit Direct-or-Goal decision surface for approved plan execution.

### Changed

- Docked effect approval review above the composer with focused summary, exact-request,
  and protection views that preserve complete sanitized scope.
- Reduced storage commit amplification during startup while retaining typed schema
  validation on the read-only fast path.
- Expanded provider connection documentation across supported hosted, compatible, and
  local model backends.

### Fixed

- Kept completion, session, theme, help, and plan-selection chrome off durable inline
  terminal scrollback by rendering focus-taking command surfaces transiently.
- Added Splunk Streamable HTTP compatibility for empty one-way acknowledgements while
  keeping credential-header exemptions scoped to the configured credential map.
- Paged resumed-session previews past tool-only records so recent user and assistant
  context remains visible in tool-heavy sessions.

### Security

- Kept dangerous full-access execution behind explicit acknowledgement while preserving
  policy, approval, audit, process-limit, and resource enforcement.
- Protected plaintext worker startup and Windows worker authentication material, and
  rejected incompatible live workers through the versioned IPC contract.

## [0.10.4] - 2026-08-04

### Fixed

- Ensured the stable draft-release job runs after the intentionally skipped Desktop
  jobs once the complete core release gate succeeds.
- Wrote Windows CLI checksum sidecars with portable LF endings so the Linux draft
  verifier and Unix consumers can validate them with `sha256sum --check`.

## [0.10.3] - 2026-08-04

### Fixed

- Allowed the exact deterministic Python source-distribution normalizer through stable
  release readiness while continuing to reject the retired root Python package and any
  other unapproved Python source.

## [0.10.2] - 2026-08-03

### Added

- Added a coordinated stable core release channel that publishes the six native CLI
  archives plus version-aligned npm, PyPI, and Go SDK releases from one source commit.
- Added immutable SDK candidate manifests and checksums, protected OIDC registry
  publishing, exact-byte recovery checks, and an independently tagged Go submodule.

### Changed

- Decoupled stable CLI and SDK releases from Apple signing, notarization, updater keys,
  and Desktop packaging. Developer Preview tags retain their explicitly unsigned
  Desktop artifacts; production Desktop distribution remains a separate release track.
- Marked every internal Rust package as non-publishable so the coordinated release does
  not accidentally expose workspace crates on crates.io.

### Release Notes

- The first stable core release requires registry trusted-publisher setup and a
  protected `sdk-production` GitHub environment before the approved draft is published.
- The Python distribution is `obscuritylabs-colossus-sdk` because the normalized
  `colossus-sdk` PyPI name belongs to an unrelated project. The import remains
  `colossus_sdk`.
- npm provenance is disabled while this repository is private because npm does not
  support provenance statements for public packages built from private repositories.

## [0.10.2-preview.10] - 2026-08-03

### Changed

- Kept YAML fenced blocks on the safe plain-text fallback so the syntax-highlighting
  dependency graph remains compatible with the supported static musl release targets.

## [0.10.2-preview.9] - 2026-08-03

### Added

- Added ChatGPT/Codex subscription authentication through the official Codex credential
  store, including browser and device-code login, status and logout commands, bounded
  token refresh, streamed Responses turns, tool calls, and model-specific reasoning
  effort levels.
- Added terminal-native transcript scrollback as the default TUI mode. Finalized user,
  assistant, tool, and system entries move into ordinary terminal history for mouse
  selection, copy, search, and wheel navigation while the composer and status remain
  sticky.
- Added CommonMark parsing and syntax-highlighted fenced code blocks to terminal
  presentation, with tables, task lists, nested emphasis, blockquotes, links, image
  fallbacks, bounded wrapping, and safe large-block fallbacks.
- Added the complete interactive Plan workflow to full-screen TUI and scripted line
  mode, with process-local Execute/Plan state, same-session Draft selection and
  refinement, approval, discard, and Direct or bounded Goal execution in embedded and
  worker-backed runtimes.
- Added `/goal resume GOAL_ID` for continuing the remaining budget of an Active goal
  after cancellation or bounded failure.
- Added typed `PlanWritten` run events and canonical plan evidence on completed and
  cancelled Plan Mode runs. Public API, protobuf, and SDK completed results and
  cancellations now expose the optional canonical `plan_id`.
- Added authenticated worker protocol v6 `RunInteractive` variants for Execute and Plan
  turns, plan lifecycle operations, Direct and Goal execution, Goal resume, notices,
  prompts, released events, and cancellation.

### Changed

- Made `--alt-screen` the explicit opt-in for the application-owned full-screen
  transcript. `--no-alt-screen` remains a compatibility alias for the default inline
  native-scrollback viewport.
- Made Ctrl-C exit an idle TUI, request cooperative cancellation for an active run, and
  exit on a second press during cancellation. `/exit` and Ctrl-D on an empty idle
  composer remain available.
- Preserved intermediate assistant output and tool boundaries as finalized transcript
  entries so native scrollback and resumed session history retain the complete visible
  run rather than only the final answer.
- Added optimistic plan revisions: legacy records default to revision 0, new plans start
  at 1, and refinement or any lifecycle transition increments the revision and rejects
  stale requests.
- Made every completed Plan Mode turn perform exactly one successful `plan.create` or
  runtime-bound `plan.update`. A missing write receives one corrective turn before
  failing closed, while duplicate writes are blocked before dispatch.
- Made approved-plan consumption atomic. Cancel or failure before consumption preserves
  terminal Plan mode and selection; after consumption, Direct and Goal outcomes retain
  canonical plan plus completion, cancellation, or bounded-failure evidence.

### Security

- Kept ChatGPT access, refresh, and account credentials out of configuration, model
  input, diagnostics, and durable run history. Codex provider access remains bound to
  the exact ChatGPT and OpenAI authentication origins through the normal effect gateway.
- Bound `plan.update` to the server-selected Draft id and revision, kept
  `plan.discard` operator-only, and routed update, discard, approval, Direct execution,
  and Goal handoff through the normal effect gateway.
- Corrected the Plan Mode inspection allowlist to `context.show` and
  `context.snapshots` while continuing to exclude filesystem writes, patch application,
  command execution, approval, networking, delegation, and plan consumption.

### Upgrade Notes

- Worker protocol v6 is intentionally incompatible with stale resident workers. Restart
  the worker and client with the same Colossus version after upgrading. Interrupted
  interactive operations are not automatically retried; inspect `/plans` and linked run
  or Goal evidence first.
- Preview upgrades are manual. Download later preview installers and their checksum
  sidecars from GitHub Releases.

## [0.10.2-preview.8] - 2026-07-30

### Fixed

- Replaced the Windows executable detachment's backup-oriented `File.Replace` call
  with the same-volume overwrite form of `File.Move`. This keeps the verified
  single-link replacement atomic without passing an empty backup path to .NET on
  the `windows-latest-l` runner image.

### Upgrade Notes

- `v0.10.2-preview.8` supersedes the unpublished `v0.10.2-preview.7` attempt.
  Preview.7 passed release readiness and compiled the Windows Desktop application,
  but Windows packaging stopped when `File.Replace` rejected its null backup-path
  argument. That attempt produced no GitHub Release.
- Preview upgrades are manual. Download later preview installers and their checksum
  sidecars from GitHub Releases.

## [0.10.2-preview.7] - 2026-07-30

### Fixed

- Detached Cargo's hard-linked top-level Windows Desktop executable with a verified,
  same-volume atomic replacement before sealing its manifest binding. The manifest
  patcher still rejects symbolic links, additional hard links, noncanonical paths, and
  oversized files.
- Kept x64 Windows acceptance, CLI packaging, and unsigned Desktop packaging on the
  exact `windows-latest-l` larger-runner label.

### Upgrade Notes

- `v0.10.2-preview.7` supersedes the unpublished `v0.10.2-preview.6` attempt. Its
  AppArmor readiness check, all six CLI targets, and macOS Desktop package passed, but
  Windows Desktop packaging correctly rejected Cargo's multiply linked PE before
  manifest binding. No earlier preview attempt produced a GitHub Release.
- Preview upgrades are manual. Download later preview installers and their checksum
  sidecars from GitHub Releases.

## [0.10.2-preview.6] - 2026-07-29

### Fixed

- Staged CI AppArmor attachment binaries in a run-unique, root-level directory so the
  exact-path profile has no runner-controlled `/usr`, `/usr/local`, or `/opt` ancestor.
  The executable and its only replaceable parent remain root-owned and non-writable by
  unprivileged users.
- Made AppArmor path and parser validation run against a harmless root-owned stub before
  compiling Colossus, so incompatible runners fail in seconds without starting release
  artifact jobs.
- Kept x64 Windows acceptance and unsigned preview packaging on the provisioned
  `windows-latest-l` larger runner.

### Upgrade Notes

- `v0.10.2-preview.6` supersedes the unpublished `v0.10.2-preview.5` attempt. Its
  ARM64 release-readiness runner exposed `/usr` as runner-controlled, so the hardened
  AppArmor installer rejected the attachment before any artifact jobs started. No
  earlier preview attempt produced a GitHub Release.
- Preview upgrades are manual. Download later preview installers and their checksum
  sidecars from GitHub Releases.

## [0.10.2-preview.5] - 2026-07-29

### Fixed

- Expanded the sealed Desktop manifest patcher's bounded executable allowance to 1 GiB
  so the stripped Windows Developer Preview PE can be bound without weakening its
  regular-file, canonical-path, or link-count checks.
- Staged AppArmor acceptance binaries below root-controlled `/usr/lib` so both x64 and
  ARM64 GitHub-hosted Linux runners satisfy the exact-path attachment policy.
- Kept x64 Windows acceptance and preview packaging on the provisioned
  `windows-latest-l` larger runner.

### Upgrade Notes

- `v0.10.2-preview.5` supersedes the unpublished `v0.10.2-preview.4` attempt. Its
  Windows Desktop executable remained above the previous 512 MiB patching ceiling, and
  its ARM64 Linux runner exposed a replaceable `/usr/local/libexec` ancestor. No earlier
  preview attempt produced a GitHub Release.
- Preview upgrades are manual. Download later preview installers and their checksum
  sidecars from GitHub Releases.

## [0.10.2-preview.4] - 2026-07-29

### Fixed

- Made Developer Preview Desktop packaging credential-free end to end: macOS is ad-hoc
  signed without notarization, Windows is unsigned, and neither preview package requires
  or emits Tauri updater signatures.
- Passed the Windows Tauri release override through a temporary JSON file so PowerShell
  and the `tauri.cmd` shim cannot strip the JSON property-name quotes.
- Applied the root workspace's stripped release profile to the standalone Tauri workspace
  so the Windows PE remains inside the sealed-manifest executable-size bound.
- Moved Windows pre-merge acceptance and preview packaging to the configured
  `windows-latest-l` larger runner.
- Disabled automatic Desktop updates for unsigned previews while preserving the signed,
  fail-closed updater contract for future stable releases.

### Upgrade Notes

- `v0.10.2-preview.4` supersedes the unpublished `v0.10.2-preview.3` attempt, whose
  standalone Tauri workspace retained symbols and produced a Windows PE larger than the
  sealed-manifest guard permits. It also supersedes `v0.10.2-preview.2`, whose inline
  JSON override lost its quotes in the PowerShell command shim, and
  `v0.10.2-preview.1`, whose unsigned path still required an updater-signing key. None of
  the failed attempts produced a GitHub Release.
- Preview upgrades are manual. Download later preview installers and their checksum
  sidecars from GitHub Releases.

## [0.10.2-preview.1] - 2026-07-29

### Fixed

- Migrated the Unix and Windows release bundle-install smoke configurations to schema
  version 2, keeping provider connections separate from model profiles so all six CLI
  artifact jobs can complete packaging verification.
- Added release-contract coverage that rejects schema version 1 in either platform
  packaging script.

### Upgrade Notes

- `v0.10.2-preview.1` supersedes the unpublished `v0.10.1` release attempt, whose
  artifact workflow stopped during packaging verification. No `v0.10.1` GitHub Release
  assets were published.
- This release remains prerelease software. Its macOS Desktop build is ad-hoc signed,
  is not Apple-notarized, and uses the archive name
  `Colossus-Desktop-DEVELOPER-PREVIEW-v0.10.2-preview.1-aarch64-apple-darwin.zip`.
  Its Windows Desktop installer is unsigned and explicitly includes `UNSIGNED` in its
  filename.

## [0.10.1] - 2026-07-28

### Added

- Added stable Apple-silicon Desktop packaging with Developer ID signing, notarization,
  stapling, Gatekeeper assessment, signed update metadata, and an explicit user-driven
  update flow.
- Added a native Windows x64 Managed Local implementation with authenticated named
  pipes, Job Object lifecycle containment, ConPTY terminals, private credential storage,
  and a separately labeled unsigned Developer Preview installer path.
- Added workspace file previews, released artifact and attachment flows, richer run
  history, capability-driven Desktop controls, and delegated workflow execution across
  the public API and SDKs.
- Added operator-supplied CA bundles across supported outbound clients without exposing
  certificate contents or private storage paths to the renderer.

### Changed

- Split provider connection profiles from model profiles in configuration schema version
  2 so model identifiers, context limits, capabilities, and logical role routing are
  explicit and independently validated.
- Refined Desktop work, settings, diagnostics, files, artifacts, compact timelines, and
  responsive keyboard and accessibility behavior for production use.
- Made interactive `colossus run` print only the released assistant response while
  preserving the structured JSON output contract for automation.
- Added a locked cross-language development container and consolidated repository checks
  behind the change-selected `cargo xtask` workflow.

### Fixed

- Hardened provider streaming, OpenAI-compatible tool calls, approval notices, partial
  failures, and terminal reconstruction so released output remains ordered, bounded, and
  useful across CLI, TUI, worker, and Desktop clients.
- Corrected Windows workspace, file-handle, private-storage, settings-migration, process,
  and release-contract behavior across supported validation runners.
- Bound context summarization prompts and repaired Desktop, release, Chroma, and
  cross-platform acceptance paths exposed during production-readiness review.

### Security

- Sanitized provider-controlled failure details before public release and excluded common
  nested Docker, Kubernetes, cloud, and application-default credential stores from
  Desktop file previews.
- Bound Desktop updater metadata, release channels, immutable artifact URLs, updater
  signatures, sealed bundle manifests, code identities, and nested executable hashes so
  stable and Developer Preview packages cannot be confused or silently substituted.
- Revalidated Windows file handles and storage ownership at use time, kept native
  credentials and CA material outside renderer authority, and preserved fail-closed
  behavior for missing stable signing or notarization configuration.

### Upgrade Notes

- Configuration schema version 1 is no longer accepted. Generate a fresh schema version
  2 file with `colossus --config PATH config init`, then deliberately transfer reviewed
  provider, model, policy, storage, and integration settings. Canonical Rust YAML and
  redb state remain authoritative; no legacy Python or SQLite state is imported.
- The stable macOS archive is
  `Colossus-Desktop-v0.10.1-aarch64-apple-darwin.zip`; verify its adjacent SHA-256
  sidecar before installation. Existing preview-era workspace bindings may require one
  explicit folder reselection before Managed Local starts.
- Windows Desktop remains an unsigned Developer Preview and is not part of the stable
  release channel. Windows CLI archives remain stable release artifacts.

## [0.10.1-preview.2] - 2026-07-22

### Fixed

- Bound the checkout-free draft-release job directly to its GitHub repository so the
  validated artifacts can be assembled into the human-approved Developer Preview draft.

### Upgrade Notes

- `v0.10.1-preview.2` supersedes `v0.10.1-preview.1` as the current Developer Preview
  candidate. It remains prerelease software; its Desktop build is ad-hoc signed, is not
  Apple-notarized, and uses the archive name
  `Colossus-Desktop-DEVELOPER-PREVIEW-v0.10.1-preview.2-aarch64-apple-darwin.zip`.

## [0.10.1-preview.1] - 2026-07-22

### Added

- Added an explicitly labeled macOS Developer Preview channel so Managed Local can be
  tested before Apple Developer ID and notarization credentials are available.
- Added a persistent in-app **Developer Preview** warning sourced from the native
  compile-time release channel rather than renderer configuration.

### Security

- Developer Preview packaging remains ad-hoc signed, preserves the sealed manifest,
  exact nested-binary hashes, fixed code identifiers, and strict pre-spawn verification,
  but makes no Apple publisher-identity or notarization claim. The stable release channel
  still requires a canonical Apple Team ID, Developer ID signing, and notarization.

### Upgrade Notes

- `v0.10.1-preview.1` is a GitHub prerelease for testing, not the stable `0.10.1`
  release. Its Desktop archive is named
  `Colossus-Desktop-DEVELOPER-PREVIEW-v0.10.1-preview.1-aarch64-apple-darwin.zip`.
- Verify the adjacent SHA-256 sidecar before opening the archive. Because this preview is
  not Apple-notarized, macOS requires an explicit Control-click **Open** or
  **System Settings → Privacy & Security → Open Anyway** confirmation. Do not disable
  Gatekeeper or strip quarantine metadata.

## [0.10.0] - 2026-07-22

### Added

- Added a versioned public gRPC API with scoped application credentials, pinned TLS
  identity, durable run and prompt operations, compatibility fixtures, and generated
  TypeScript, Python, and Go SDKs alongside the native Rust SDK.
- Added Colossus Operations Studio, a dark blue Tauri workspace for individual work,
  multi-target fleet health, released artifacts, activity, settings, and sanitized
  Markdown run transcripts.
- Added Managed Local as the folder-first Desktop path: a supervised macOS sidecar,
  native provider enrollment into the system keychain, workspace-scoped lifecycle,
  bounded restart recovery, and authenticated parity with external daemon targets.
- Added opt-in local shell tabs and a one-click Colossus TUI that attaches to the existing
  managed worker, plus a unified access-profile model for consistent CLI, TUI, worker,
  public API, and Desktop authority.
- Added the workspace development shell with explicit filesystem and network policy,
  native sandbox initialization, and protected-shell handling on Linux and macOS.

### Changed

- Split hosted validation into fail-closed, path-classified PR and pre-merge tiers while
  retaining stable aggregate gates; release automation now builds six native CLI targets
  and a separately signed and notarized Apple-silicon Desktop artifact.
- Refactored runtime, worker, CLI, TUI, workflow, presentation, sandbox, pack, and service
  crate roots into focused composition modules, and migrated the documentation site to
  Zensical with redesigned product and architecture guidance.
- Made durable queue insertion wake workers immediately so newly queued and resumed work
  does not wait for the polling interval.

### Fixed

- Made Rust public-API code generation use an exact cross-platform vendored `protoc` so
  clean developer, CI, and release runners do not depend on an ambient compiler install.
- Shortened overlong macOS worker socket paths through an owner-private, deterministic
  lease endpoint while preserving direct legacy endpoints wherever the operating system
  accepts them.
- Bound workspace ownership to platform process identity, capped aggregate skill and pack
  discovery resources, and preserved graceful run draining, transport force-close, and
  checkpoint shutdown across managed-runtime exits.

### Security

- Managed Local verifies the bundle manifest and macOS code identity immediately before
  no-shell process creation, transfers one-use credentials and provider secrets only over
  a bounded inherited channel, and fails closed on identity, TLS, API, grant, workspace,
  or writer-lease mismatch.
- Provider secrets are resolved from native memory only after policy authorization,
  zeroized after use, and excluded from renderer DTOs, configuration, environment,
  command arguments, logs, terminal sessions, telemetry, and run context.
- Desktop terminals accept only native-selected shell or TUI sessions for opaque
  workspace handles, isolate capabilities in a dedicated local WebView, bound all I/O,
  disable escape-sequence clipboard writes and automatic navigation, and guarantee
  process-tree cleanup.
- Public API and SDK clients authenticate every connection with a certificate pin,
  instance identity, API compatibility check, scoped bearer, exact role/tool grants, and
  bounded watch limits; administrative and delegated authority remain excluded.

### Upgrade Notes

- Existing daemon installations and external Desktop targets remain supported without
  migration. New Desktop installations default to Managed Local and keep app-owned state
  outside the selected repository.
- Managed Local and signed Desktop distribution are macOS-first in this release. Other
  platforms continue to use the CLI, TUI, daemon, and public SDK surfaces until their
  native terminal and packaging backends land.
- The public API is still `v1alpha1`, and the TypeScript and Python SDK packages remain
  alpha packages; package publication is separate from this source and binary release.

## [0.9.0] - 2026-07-17

### Added

- Added provider-neutral `web.search` with explicit SearXNG or SerpAPI profiles, exact
  `agent` and `research` role routing, normalized bounded results, operator diagnostics,
  and shared agent/Deep Research execution.
- Added reproducible signed collections for packs and data-only skills, including
  independent publisher verification, complete dependency-closure validation,
  no-clobber batch installation, and CLI, TUI, worker, and embedded runtime operations.
- Added authenticated registry pull and create-only push for signed collections using
  deterministic bounded archives and optional environment-backed credentials.

### Changed

- Consolidated remaining product scope into the feature inventory and removed the
  superseded standalone roadmap.
- Made the default provider-neutral search user agent identify the 0.9 release line.

### Security

- `web.search` now crosses the ordinary effect gateway with an exact origin, DNS pinning,
  redirects and ambient proxies disabled, late credential resolution, mandatory
  post-effect authorization, and credential removal before normalization or release.
- Collection and registry operations reject traversal, links, special or undeclared
  entries, signature or hash mismatches, incomplete pack dependency closure, destination
  clobbering, and unapproved network or filesystem access.

### Upgrade Notes

- Existing configurations remain valid. The deprecated 0.8 `research.search` SearXNG
  form is accepted only when top-level `search` is absent; new `web.search` use requires
  an explicit role mapping and never falls back to another provider.
- Existing state and installed packs are not migrated or replaced automatically.
  Collections and registry transport are opt-in, and installation refuses existing
  destinations.

## [0.8.1] - 2026-07-16

### Added

- Added `scripts/colossus-dev`, an isolated source-development launcher that reuses the
  debug build, loads owner-only environment keys only after compilation, and keeps its
  configuration, redb state, and secure anchor separate from operator state.

### Changed

- Rotated the official Ed25519 offline-bundle publisher identity and added `.env` to
  ignored local files so an operator-held signing seed cannot be accidentally staged.

### Security

- The `v0.8.0` source tag remains immutable, but no signed GitHub release was published
  from it after the previous private seed became unavailable. Version 0.8.1 establishes
  a new exact publisher/key binding and is the first signed distribution of the 0.8
  release line.
- Operators who trusted the earlier publisher identity must verify the new
  `release/bundle-publisher.json` through an independent channel and explicitly replace
  the old trust binding before installing the 0.8.1 signed bundle.

## [0.8.0] - 2026-07-16

### Added

- Added durable fixed-cadence workflow schedules with deterministic skip or fire-once
  misfire handling, hash-pinned definitions, atomic queued-run creation, and
  process-kill-safe recovery.
- Added HMAC-SHA256 authenticated workflow webhooks with bounded replay protection,
  exact raw-body verification, policy and audit routing, atomic delivery receipts, and
  loopback HTTP, CLI, TUI, and worker surfaces.
- Added exact repository-domain-event workflow subscriptions with optional stream
  scoping, durable checkpoints, deterministic run identities, duplicate-delivery
  suppression, and deferred policy-refusal handling.
- Added a transactional PostgreSQL journal and projection adapter with encrypted
  envelopes, global-chain serialization, shared repository conformance, TLS policy,
  credential-reference-only configuration, and concurrent-process recovery tests.
- Added permit-bound HTTPS WORM audit export with deterministic create-only objects,
  exact-origin networking, credential redaction, idempotent replay, and durable unknown
  outcome handling.
- Added a searchable Rust-native mdBook documentation site deployed from `main`, with
  executable navigation, link, and Pages-permission contracts.

### Changed

- Parallelized the pull-request and release validation matrices behind a fast formatting
  and locked-metadata preflight, added shared compiler caching, removed redundant native
  workspace compilation, and replaced source-built supply-chain tools with pinned,
  checksum-verified release binaries.
- Expanded the storage configuration to select redb or PostgreSQL explicitly while
  preserving redb as the local default and canonical configuration behavior.
- Updated workflow, storage, security, operator, and reconstruction documentation for
  the new durable trigger and external-storage boundaries.

### Fixed

- Hardened Windows worker named-pipe backlog, replacement-listener, routing, shutdown,
  and contention behavior so a busy authenticated worker is never mistaken for an
  absent worker that permits a second embedded writer.
- Granted the documentation workflow the Pages permission required by its pinned setup
  action and added regression coverage for that deployment boundary.
- Scoped `sccache` environment variables to jobs that install the wrapper, preventing
  dependency-policy jobs from attempting to invoke a missing compiler wrapper.
- Increased the bounded OCI control-command startup allowance so cold rootless Podman
  initialization does not spuriously fail the same security acceptance suite that
  Docker completes.
- Reused the production RustSec database for the fuzz lockfile audit, removing a
  redundant network fetch from the fail-closed supply-chain job.
- Injected both standard Unix proxy-variable spellings for native and OCI sandboxes so
  clients such as curl use the authenticated allowlist proxy on every release platform.

### Security

- Workflow triggers revalidate definition hashes, call graphs, input schemas, replay
  state, and policy immediately before atomic dispatch; trigger creation grants no
  downstream effect authority.
- PostgreSQL and WORM credentials remain late-resolved environment references, verified
  TLS is required outside explicit loopback acceptance, and diagnostics never release
  connection strings, bearer values, ciphertext, or plaintext audit payloads.

### Upgrade Notes

- Existing redb installations remain redb installations. Selecting PostgreSQL is an
  explicit configuration change and does not silently import, copy, or replace local
  state; operators must provision and validate the target independently.
- Schedule, webhook, and subscription records are additive. Existing workflow
  definitions and runs retain their current hashes and behavior until an operator
  explicitly creates and enables a trigger.
- Signed multi-pack collections and authenticated remote pack-registry pull/push
  operations are intentionally deferred to the 0.9.0 roadmap. Local verified packs,
  OCI layouts, and signed offline release bundles remain supported in 0.8.0.

## [0.7.0] - 2026-07-15

### Added

- Added a Ratatui terminal with a durable paged and reflowing transcript, pinned composer
  and footer, modal overlays, queued input, encrypted history, and terminal restoration
  across resize, cancellation, panic, and normal exit paths.
- Added width-aware semantic tables, status and result blocks, Markdown, source previews
  with line numbers, styled diffs, separated process streams, and compact borderless
  rendering for large resource and tool listings.
- Added visible slash-command and discovered `@skill` completion menus, fish-style ghost
  text, a guided theme picker, complete semantic theme previews, and the tested custom
  Ocean theme.
- Added authenticated worker protocol v4 frames for approval decisions, user input, and
  cooperative cancellation with embedded and worker-host parity.
- Added `--output auto|human|json`; terminals default to human output while pipes retain
  stable JSON for automation.

### Changed

- Made the Ratatui interface the sole interactive terminal owner for `colossus` and
  `colossus tui`, while preserving non-TTY line mode and explicit JSON automation.
- Replaced raw structured dumps with intentional list/detail views, explicit empty
  states, brighter readable theme text, and color-separated prompts, answers, reasoning,
  tools, warnings, and errors.
- Routed session resume, approvals, and `user.ask` through stable foreground TUI flows
  with explicit answer and cancellation guidance.
- Woke the bounded scheduler immediately for model-created subagents so a parent can
  receive the completed child result in the same turn, and rendered queued/running
  results as pending instead of failed.
- Replaced nested theme JSON with an active-state table and readable custom-theme search
  locations; selection now saves immediately and updates the complete TUI palette.

### Removed

- Removed the superseded public `repl` subcommand and its competing terminal ownership
  path. Durable journal compatibility terminology remains internal only.

## [0.6.0] - 2026-07-14

### Added

- Auditable Rust agent and workflow runtime with an encrypted event journal, policy
  gateway, durable sessions/workflows, memory indexes, sandboxed effects, and native
  distribution tooling.
- OpenAI Responses, OpenAI-compatible, and credential-free echo providers with CLI,
  REPL, worker, and embedded runtime surfaces.
- Model-assisted `risk-auto` review for approval-required shell requests. Only a strict
  low-risk allow result can create an automatic approval proof; all other results and
  evaluator failures return control to the user.
- Release-readiness documentation for installation, configuration, offline and
  airgapped operation, bundle format, release process, and security policy.
- Continuous integration covering formatting, linting, tests, fuzzing, supply-chain
  policy, live security adapters, and six native release targets.

### Changed

- Promoted the Rust workspace to the repository root and made Rust 1.96/edition 2024 the
  active build contract.
- Replaced the Python-dependent commit checker, development container, Docker image, and
  CI layout with Rust-root equivalents.
- Renamed the transitional `colossus-rs` executable to the canonical `colossus` command
  used by installed, container, and release artifacts.
- Added a reproducible host-side cutover verifier that pins Rust and supply-chain tools,
  rejects reintroduced Python source, and checks both production and fuzz dependency
  graphs.
- Split hosted validation into an inexpensive post-merge Ubuntu gate, a fail-closed pull
  request test/security gate, and an explicit six-target release gate.

### Fixed

- Preserved the exact persisted event representation during journal hash verification
  and authenticated decryption so additive context fields do not invalidate older Rust
  journal records.
- Cached platform credential material per service/account for the process lifetime so
  journal replay and concurrent runtime setup do not repeatedly reopen the same Keychain,
  DPAPI, or Secret Service entry; failed credential reads remain uncached.
- Hardened authenticated worker IPC and Windows named-pipe retries so canonical response
  payloads remain authenticated under contention without weakening timeout behavior on
  other platforms.
- Parsed protocol-skill frontmatter identically with LF or CRLF line endings so native
  Windows archive and signed-bundle installations load the bundled skill library.

### Removed

- Removed the Python 0.5 runtime, tests, packaging, and SQLite state contract from
  `main`; the frozen implementation remains at `python-v0.5.0` and on `python-legacy`.

## [0.1.0] - 2026-06-08

### Added

- Initial secure layered CLI harness with CLI, REPL, and TUI interfaces.
- Ports-and-adapters architecture with dependency-inward boundaries.
- Deterministic echo provider and OpenAI-compatible provider adapters.
- Brokered built-in tool metadata, policy decisions, local state, and audit logging.
- Bundled skills and offline bundle manifest verification.
