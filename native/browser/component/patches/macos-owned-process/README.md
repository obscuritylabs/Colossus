# Pinned owned-process compatibility patch

`manifest.json` locks the exact Chromium `154.0.8037.98` input bytes, CEF commit
`14c5a089e8452874cb1a556dc3fde26ed2d87723`, combined patch and fixed client overlay.
The process changes delegate the supported Mac Main/helper creation and status
operations to the authenticated native creator. Component receipts separately
require the actual built private loader symbols and every front/body artifact;
source patch applicability alone does not assert native acceptance.

The patch also repairs a startup interaction with that CEF commit's upstream
[`base_rand_util_lazy_seed.patch`](https://github.com/chromiumembedded/cef/blob/14c5a089e8452874cb1a556dc3fde26ed2d87723/patch/patches/base_rand_util_lazy_seed.patch).
CEF makes the metrics PRNG seed lazily through BoringSSL, which may allocate.
Chromium's lock recorder samples inside PartitionAlloc's nonallocating critical
path. Its original default constructor leaves the sampler unseeded, so the first
lock sample can recursively allocate and trigger the thread-cache reentrancy
check. This was observed during an actual owned CEF startup on macOS.

The constructor primes the sampler while explicit initialization permits
allocation. `EnableRecordingOnCurrentThread` completes `make_unique` construction
before `tls_slot->Set` publishes the recorder object. The allocation operation
for the object has returned before the C++ constructor runs; its thread-cache
guard has already left scope. The earlier global TLS-slot pointer is not a
published recorder on this thread. No allocator check, sandbox, entropy provider
or telemetry feature is disabled. The upstream CEF patch SHA256 is recorded in
the source and native receipts and checked against its pinned Git object.

With the independently verified source checkout, run the CEF-free regression:

```sh
COLOSSUS_PINNED_CHROMIUM_SOURCE=/absolute/pinned/chromium/src \
  python3 -B -m unittest discover -s native/browser/scripts \
  -p test_owned_lock_metrics_seed.py -v
```

The pure harness compiles the actual pinned lazy RNG/sampler and patched recorder
method bodies against a synthetic allocating entropy provider. The original
constructor detects forbidden entropy allocation on the first guarded sample;
the repaired constructor seeds before publication and permits 1,000 guarded
samples without another entropy allocation. It loads no CEF and launches no
browser, Mach fixture or process supervisor. Only a separate actual native run
can prove that the linked framework initializes and renders a page.
