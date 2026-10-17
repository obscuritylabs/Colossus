"""CEF-free regression using exact pinned sampler and recorder method bodies.

Set COLOSSUS_PINNED_CHROMIUM_SOURCE to the checked source used for the owned
framework. The compiled pure harness has no CEF, Mach, browser or child launcher;
its synthetic entropy provider rejects allocation during a guarded hot path.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import owned_process_component as owned


def method(source, marker):
    start = source.index(marker)
    brace = source.index("{", start)
    depth = 0
    for index in range(brace, len(source)):
        if source[index] == "{":
            depth += 1
        elif source[index] == "}":
            depth -= 1
            if depth == 0:
                return source[start:index + 1]
    raise ValueError("pinned method body is incomplete")


CONTEXT = r'''
#include <atomic>
#include <cassert>
#include <cstddef>
#include <cstdint>
#include <cstdlib>
#include <memory>
#include <new>
#include <string>
#include <string_view>
struct ReentrantAllocation {};
bool allocator_active = false;
bool recorder_published = false;
unsigned entropy_calls = 0;
struct AllocatorGuard {
  AllocatorGuard() { assert(!allocator_active); allocator_active = true; }
  ~AllocatorGuard() { allocator_active = false; }
};
void* operator new(std::size_t size) {
  if (allocator_active) throw ReentrantAllocation{};
  AllocatorGuard guard;
  if (void* result = std::malloc(size)) return result;
  throw std::bad_alloc();
}
void operator delete(void* value) noexcept { std::free(value); }
#define DCHECK(value) assert(value)
namespace base {
std::atomic<bool> g_subsampling_always_sample{false};
std::atomic<bool> g_subsampling_never_sample{false};
uint64_t RandUint64() {
  // Entropy allocation is allowed during explicit construction only. It must
  // never occur with a PartitionAlloc-like guard active or after publication.
  if (allocator_active) throw ReentrantAllocation{};
  assert(!recorder_published);
  auto allocation = std::make_unique<unsigned char>(7);
  return ++entropy_calls + *allocation;
}
class InsecureRandomGenerator {
 public:
  InsecureRandomGenerator();
  uint64_t RandUint64() const;
  double RandDouble() const;
 private:
  mutable uint64_t a_ = 0, b_ = 0;
};
class MetricsSubSampler {
 public:
  MetricsSubSampler();
  bool ShouldSample(double probability) const;
 private:
  InsecureRandomGenerator generator_;
};
class LockMetricsRecorder {
 public:
  struct PassKey {};
  LockMetricsRecorder(PassKey, std::string_view histogram_suffix);
  bool ShouldRecordLockAcquisitionTime() const;
  bool CalledOnValidThread() const { return true; }
 private:
  static constexpr double kSamplingRatio = 0.005;
  std::string histogram_suffix_;
  bool iterating_in_progress_ = false;
  MetricsSubSampler subsampler_;
};
'''


@unittest.skipUnless(os.environ.get("COLOSSUS_PINNED_CHROMIUM_SOURCE"),
                     "requires the independently checked exact pinned source checkout")
class PinnedSamplerRegression(unittest.TestCase):
    def test_constructor_seeds_before_publication_and_first_guarded_sample_allocates_nothing(self):
        root = Path(os.environ["COLOSSUS_PINNED_CHROMIUM_SOURCE"]).resolve(strict=True)
        manifest = json.loads((owned.PATCH / "manifest.json").read_text())
        upstream = root / "cef/patch/patches/base_rand_util_lazy_seed.patch"
        self.assertEqual(hashlib.sha256(upstream.read_bytes()).hexdigest(),
                         manifest["cef_lazy_sampler_seed_patch_sha256"])
        random = (root / "base/rand_util.cc").read_text()
        recorder = (root / "base/synchronization/lock_metrics_recorder.cc").read_text()
        constructor = method(recorder, "LockMetricsRecorder::LockMetricsRecorder(")
        methods = ["InsecureRandomGenerator::InsecureRandomGenerator() = default;",
                   method(random, "uint64_t InsecureRandomGenerator::RandUint64() const"),
                   method(random, "double InsecureRandomGenerator::RandDouble() const"),
                   "MetricsSubSampler::MetricsSubSampler() = default;",
                   method(random, "bool MetricsSubSampler::ShouldSample(double probability) const"),
                   method(recorder, "bool LockMetricsRecorder::ShouldRecordLockAcquisitionTime() const")]
        baseline_source = subprocess.check_output(
            ["git", "show", "HEAD:base/synchronization/lock_metrics_recorder.cc"],
            cwd=root, text=True)
        self.assertEqual(hashlib.sha256(baseline_source.encode()).hexdigest(),
                         manifest["source_sha256"]["base/synchronization/lock_metrics_recorder.cc"])
        baseline = method(baseline_source, "LockMetricsRecorder::LockMetricsRecorder(")
        main = r'''
}
int main() {
  // The actual C++ new-expression returns from operator new (and its allocator
  // guard) before invoking the real constructor, just like make_unique at the
  // pinned EnableRecordingOnCurrentThread call site before tls_slot->Set.
  auto recorder = std::make_unique<base::LockMetricsRecorder>(
      base::LockMetricsRecorder::PassKey{}, "CrBrowserMain");
  assert(entropy_calls == EXPECTED_INITIAL_ENTROPY_CALLS);
  recorder_published = true;
  bool rejected_reentry = false;
  try {
    AllocatorGuard guarded_sampling;
    for (unsigned i = 0; i < 1000; ++i)
      (void)recorder->ShouldRecordLockAcquisitionTime();
  } catch (const ReentrantAllocation&) {
    rejected_reentry = true;
  }
  assert(rejected_reentry == EXPECTED_REENTRY);
  assert(entropy_calls == EXPECTED_INITIAL_ENTROPY_CALLS);
}
'''
        compiler = shutil.which("clang++")
        self.assertIsNotNone(compiler)
        with tempfile.TemporaryDirectory(prefix="owned-sampler-pure-") as temporary:
            directory = Path(temporary)
            for variant, source, calls, reentry in (("baseline", baseline, 0, "true"),
                                                     ("repaired", constructor, 2, "false")):
                with self.subTest(variant=variant):
                    path = directory / (variant + ".cc")
                    path.write_text(CONTEXT + "\n" + "\n".join(methods) + "\n" + source + main)
                    executable = directory / variant
                    subprocess.run([compiler, "-std=c++20", "-Wall", "-Wextra", "-Werror",
                                    f"-DEXPECTED_INITIAL_ENTROPY_CALLS={calls}",
                                    f"-DEXPECTED_REENTRY={reentry}", str(path), "-o", str(executable)],
                                   check=True, capture_output=True, timeout=30)
                    subprocess.run([str(executable)], check=True, capture_output=True, timeout=10)


if __name__ == "__main__":
    unittest.main()
