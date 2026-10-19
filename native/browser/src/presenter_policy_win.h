#ifndef COLOSSUS_PRESENTER_POLICY_WIN_H
#define COLOSSUS_PRESENTER_POLICY_WIN_H
#include <cstdint>

namespace colossus {
// A completed image is display data, never an input or Core ownership lease.
constexpr bool PresenterRoutesInput(bool snapshot, bool live, bool visible, bool foreground) {
  return !snapshot && live && visible && foreground;
}
constexpr bool SnapshotDisplayPulse(uint64_t previous, uint64_t next, uint32_t ttl) {
  return next > previous && ttl > 0 && ttl <= 1500;
}
}
#endif
