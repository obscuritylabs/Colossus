#include "colossus_cef.h"
#include "synthetic_profile.h"

#if defined(COLOSSUS_CEF_MAC_SYNTHETIC_PROFILE_DIAGNOSTIC)
#if !defined(__APPLE__) || defined(NDEBUG)
#error "Synthetic macOS profiles require an explicit macOS Debug diagnostic build"
#endif
#include <atomic>
#include <pthread.h>
#endif

namespace colossus {
#if defined(COLOSSUS_CEF_MAC_SYNTHETIC_PROFILE_DIAGNOSTIC)
namespace {
enum class Phase { Unselected, Prepared, Consumed };
std::atomic<Phase> phase{Phase::Unselected};
}
#endif

const char* SyntheticProfileSwitch(bool browser_process) {
#if defined(COLOSSUS_CEF_MAC_SYNTHETIC_PROFILE_DIAGNOSTIC)
  if (browser_process && phase.load() != Phase::Unselected)
    return "use-mock-keychain";
#else
  (void)browser_process;
#endif
  return nullptr;
}

int32_t SyntheticProfileBootstrap(int32_t mode) {
#if defined(COLOSSUS_CEF_MAC_SYNTHETIC_PROFILE_DIAGNOSTIC)
  if (!pthread_main_np()) return COLOSSUS_CEF_WRONG_THREAD;
  if (mode != 2) return COLOSSUS_CEF_DENIED;
  auto expected = Phase::Prepared;
  if (!phase.compare_exchange_strong(expected, Phase::Consumed))
    return expected == Phase::Consumed ? COLOSSUS_CEF_BUSY : COLOSSUS_CEF_UNAVAILABLE;
#else
  (void)mode;
#endif
  return COLOSSUS_CEF_OK;
}

bool SyntheticProfileAllowsPersistence() {
#if defined(COLOSSUS_CEF_MAC_SYNTHETIC_PROFILE_DIAGNOSTIC)
  return false;
#else
  return true;
#endif
}
}

extern "C" int32_t colossus_cef_synthetic_profile_prepare(void) {
#if defined(COLOSSUS_CEF_MAC_SYNTHETIC_PROFILE_DIAGNOSTIC)
  if (!pthread_main_np()) return COLOSSUS_CEF_WRONG_THREAD;
  auto expected = colossus::Phase::Unselected;
  if (!colossus::phase.compare_exchange_strong(expected, colossus::Phase::Prepared))
    return COLOSSUS_CEF_BUSY;
  return COLOSSUS_CEF_OK;
#else
  return COLOSSUS_CEF_UNAVAILABLE;
#endif
}
