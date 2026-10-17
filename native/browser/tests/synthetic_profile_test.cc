// Configuration-only test. Never initialize CEF/AppKit or call a Keychain API.
#include "colossus_cef.h"
#include "synthetic_profile.h"
#include <cstdio>
#include <cstring>
#include <thread>

namespace {
bool expect(bool value, const char* contract) {
  if (!value) std::fprintf(stderr, "FAIL synthetic profile contract: %s\n", contract);
  return value;
}
}

int main() {
  bool passed = expect(colossus::SyntheticProfileSwitch(true) == nullptr,
                       "no startup switch before native preparation");
  passed &= expect(colossus::SyntheticProfileSwitch(false) == nullptr,
                   "helpers never select a mock provider");
#if defined(COLOSSUS_EXPECT_SYNTHETIC_PROFILE)
  char* empty_arguments[] = {nullptr};
  colossus_cef_bootstrap_options options{};
  options.abi_version = COLOSSUS_CEF_ABI_VERSION;
  options.argv = empty_arguments;
  options.headless = 2;
  int32_t subprocess_exit = -1;
  // These ABI denials happen before framework loading or CEF initialization.
  passed &= expect(colossus_cef_bootstrap(&options, &subprocess_exit) == COLOSSUS_CEF_UNAVAILABLE,
                   "public bootstrap cannot enter eager crypto without preparation");
  passed &= expect(colossus::SyntheticProfileBootstrap(2) == COLOSSUS_CEF_UNAVAILABLE,
                   "diagnostic bootstrap requires explicit preparation");
  int32_t worker_status = COLOSSUS_CEF_OK;
  std::thread worker([&] { worker_status = colossus_cef_synthetic_profile_prepare(); });
  worker.join();
  passed &= expect(worker_status == COLOSSUS_CEF_WRONG_THREAD,
                   "off-main preparation cannot select the provider");
  passed &= expect(colossus_cef_synthetic_profile_prepare() == COLOSSUS_CEF_OK,
                   "explicit one-use main-thread selection");
  passed &= expect(colossus_cef_synthetic_profile_prepare() == COLOSSUS_CEF_BUSY,
                   "selection cannot be replaced or repeated");
  const auto* selection = colossus::SyntheticProfileSwitch(true);
  passed &= expect(selection && std::strcmp(selection, "use-mock-keychain") == 0,
                   "exact pinned Chromium testing switch");
  passed &= expect(colossus::SyntheticProfileSwitch(false) == nullptr,
                   "selected fixture still excludes helper callbacks");
  passed &= expect(colossus_cef_profile_persistent(1) == COLOSSUS_CEF_DENIED,
                   "synthetic encryption cannot be persisted");
  passed &= expect(colossus_cef_profile_persistent(0) == COLOSSUS_CEF_OK,
                   "temporary request contexts remain supported");
  options.headless = 0;
  passed &= expect(colossus_cef_bootstrap(&options, &subprocess_exit) == COLOSSUS_CEF_DENIED,
                   "public bootstrap rejects native-child placement before CEF");
  passed &= expect(colossus::SyntheticProfileBootstrap(0) == COLOSSUS_CEF_DENIED &&
                   colossus::SyntheticProfileBootstrap(1) == COLOSSUS_CEF_DENIED,
                   "diagnostic supports only separate-host OSR mode");
  passed &= expect(colossus::SyntheticProfileBootstrap(2) == COLOSSUS_CEF_OK &&
                   colossus::SyntheticProfileBootstrap(2) == COLOSSUS_CEF_BUSY,
                   "one process initialization consumes the selection");
#else
  passed &= expect(colossus_cef_synthetic_profile_prepare() == COLOSSUS_CEF_UNAVAILABLE,
                   "ordinary component has no synthetic selection");
  passed &= expect(colossus::SyntheticProfileBootstrap(0) == COLOSSUS_CEF_OK &&
                   colossus::SyntheticProfileBootstrap(2) == COLOSSUS_CEF_OK &&
                   colossus::SyntheticProfileAllowsPersistence(),
                   "ordinary bootstrap behavior is unchanged");
#endif
  if (passed) std::puts("PASS synthetic profile selection; browser=false keychain=false production=false");
  return passed ? 0 : 1;
}
