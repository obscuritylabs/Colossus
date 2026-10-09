# CEF component provisioning

The reviewed source owner is [cef.lock.json](cef.lock.json). It pins the minimal
CEF distribution for Linux x64, Windows x64, macOS x64, and macOS ARM64 to
`154.0.34+g14c5a08+chromium-154.0.8037.98`. Each SHA-256 was computed from the
downloaded archive after verifying its size and SHA-1 against the upstream CEF
build index. The recorded SHA-1 is provenance metadata; acquisition accepts only
the pinned SHA-256 and size.

This provisioning tool is for developers and release builders. Colossus never
runs it to acquire a browser during a session. An installed distribution must
contain its browser, helpers, framework/libraries, resources, licenses, and
integrity evidence already.

## Acquire sources

From the repository root, using Python 3.11 or later:

```sh
python3 -B native/browser/scripts/component.py fetch --platform linux64
```

Replace `linux64` with `windows64`, `macosx64`, or `macosarm64` for the native build
target. The command prints the verified source directory for `CEF_ROOT`. Its
default cache is `.local/cef-cache`; `--cache DIRECTORY` selects a developer
cache. Archives come exclusively from their exact pinned HTTPS archive URLs at
`cef-builds.spotifycdn.com`. TLS verification remains enabled, redirects remain
on the same archive URL, downloads are size/time bounded, and failed downloads
are removed. No URL override is accepted.

Extraction rejects path escapes, duplicate and case-colliding names, hard links,
special files, and escaping, missing, cyclic, or nested-write symlinks. Internal
framework symlinks are preserved. Source file permissions are normalized to
`0644` or `0755`; directories are `0755`. Existing extracted sources are checked
against content hashes derived from the verified archive, so an adjacent source
receipt cannot authorize substituted source bytes. Keep build products outside
the extracted tree. A modified cache fails closed; select a fresh private cache
or deliberately remove that developer cache after inspecting it.

## Inventory an installed component

After building and staging the native component, inventory it:

```sh
python3 -B native/browser/scripts/component.py inventory \
  --root .local/browser-install --platform linux64 \
  --executable colossus-browser-probe
python3 -B native/browser/scripts/component.py verify --root .local/browser-install
```

The executable must be a relative regular file within the component. Inventory
records every directory and its permissions, every regular file and its SHA-256,
size and permissions, and every symlink's target and target-text SHA-256. It
rejects hard-linked regular files, special files, setuid/setgid files, unsafe link
graphs and unreadable trees. Verification rejects missing, added, modified, or
permission-changed entries. The inventory command refuses to overwrite an
existing manifest: restage after a build or signing change.

`browser-component.json` has schema and protocol version `1`, identifies the
CEF/Chromium versions, platform and target, source archive digest and relative
executable, and includes the complete `files` inventory. Its `modes` object is
always `{"desktop": false, "headless": false}` in this initial component.
Compilation or a passing inventory check does not establish native acceptance.
There is no switch to promote modes to supported. Native acceptance and a
reviewed, separately authenticated release gate must precede promotion.

The adjacent inventory establishes consistency, not publisher identity. Runtime
loaders must bind the manifest digest and component contents to the host's trusted
release provenance/signature before executing them. A component and a recomputed
adjacent inventory cannot authorize their own execution. The installed component
belongs in an immutable distribution location; writable session/profile data
belongs in a separate owner-private directory. Inventory must run after every
binary signing or bundle transformation that changes the staged bytes.

## Preserve the Chromium sandbox

CEF requires native process bootstrap and packaging in addition to copying its
main library. Keep the exact helpers/resources for the pinned platform:

- Windows requires CEF's bootstrap executable and `RunConsoleMain` or
  `RunWinMain` exported by the client DLL. The bootstrap passes its initialized
  `sandbox_info` to the client. The client must pass that information to
  `CefExecuteProcess` and `CefInitialize`; never substitute a null sandbox
  context. Use the pinned distribution's library loader/version checks and keep
  its DLLs and resources beside the bootstrap executable. Sign the bootstrap,
  client DLL, CEF DLLs and dependent executables before sealing the inventory.
- macOS requires a browser app bundle, its CEF framework, and appropriately
  identified helper app bundles. Each helper initializes
  `CefScopedSandboxContext` before loading CEF with
  `CefScopedLibraryLoader::LoadInHelper`; the main process uses `LoadInMain`.
  Dynamic loading is required by CEF's macOS sandbox. Nested helpers/frameworks
  require their proper entitlements and signatures before outer bundle signing
  and notarization. A flat executable next to a framework is not an accepted
  macOS distribution.
- Linux requires the CEF libraries, `chrome-sandbox`, resources and locales, plus
  host support for Chromium's user-namespace/seccomp sandbox. This inventory
  utility does not elevate privileges or install a root-owned setuid helper; it
  rejects setuid/setgid payloads. A host that cannot use the validated namespace
  sandbox must report unsupported rather than retrying with sandboxing disabled.
  The glibc CEF component is separate from Colossus's musl CLI binaries and does
  not establish support for Linux ARM64, Alpine, or a container host by itself.

The component must not set `no_sandbox`, append `--no-sandbox`, fall back to an
ambient Chrome installation, or expose an unauthenticated remote debugging port.
Direct control goes through the Colossus-owned native protocol and runtime
authority boundary. Headless operation retains the same sandbox requirements.

## Verify changes

The provisioning integrity suite uses only the Python standard library and
opens no network connection:

```sh
python3 -B -m unittest discover -s native/browser/scripts -p test_component.py -v
```

Its fixtures cover archive path/link attacks, digest mismatches, redirect bounds,
source-cache substitution, installed-file tampering, unexpected entries,
permission changes and unsupported mode promotion. Native build and browser
acceptance remain separate checks.

## Exercise the native no-display fixture

After building and installing `colossus-browser-probe`, inventory the final stage
and run the developer Linux x64 harness:

```sh
python3 -B native/browser/scripts/component.py inventory \
  --root .local/cef-component --platform linux64 \
  --executable colossus-browser-probe
python3 -B native/browser/scripts/run_probe.py --component .local/cef-component
```

The harness requires a verified component inventory. It serves only the tracked
`native/browser/tests/fixture.html` on an ephemeral `127.0.0.1` listener, starts
the absolute probe executable in a fresh temporary session directory, and removes
`DISPLAY` and `WAYLAND_DISPLAY`. Existing proxy, TLS trust and other environment
configuration is preserved. It downloads no engine and introduces no sandbox
override. The process is limited to 30 seconds and a combined 64 KiB of output;
its owned CEF process group is terminated on deadline/output failure.

Acceptance requires exit zero, a successful DevTools command, the native DOM/input/
screenshot fixture marker, and the stale-generation/denied-origin/malformed-command
negative-control marker. A restricted Linux command sandbox may prohibit Chromium
user namespaces even while ordinary fixture HTTP works. Test on a compatible
native runner rather than disabling Chromium's sandbox. A passing probe still
leaves both release mode flags false: it does not establish Windows/macOS Desktop
embedding, general browser egress containment, PKI acceptance, or signed packaging.

The harness's subprocess/HTTP tests use fake executables and need loopback sockets,
but do not start Chromium:

```sh
python3 -B -m unittest discover -s native/browser/scripts -p 'test_*.py' -v
```
