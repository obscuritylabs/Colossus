#include "presenter_policy_win.h"
#include <limits>

int main() {
  using colossus::PresenterRoutesInput;
  using colossus::SnapshotDisplayPulse;
  for (unsigned flags = 0; flags != 8; ++flags) {
    if (PresenterRoutesInput(true, (flags & 1) != 0, (flags & 2) != 0, (flags & 4) != 0)) return 1;
  }
  if (!PresenterRoutesInput(false, true, true, true)) return 2;
  if (PresenterRoutesInput(false, false, true, true) ||
      PresenterRoutesInput(false, true, false, true) ||
      PresenterRoutesInput(false, true, true, false)) return 3;
  if (!SnapshotDisplayPulse(0, 1, 1500) || !SnapshotDisplayPulse(7, 8, 1)) return 4;
  if (SnapshotDisplayPulse(0, 0, 1500) || SnapshotDisplayPulse(7, 7, 1500) ||
      SnapshotDisplayPulse(8, 7, 1500) || SnapshotDisplayPulse(0, 1, 0) ||
      SnapshotDisplayPulse(0, 1, 1501) ||
      SnapshotDisplayPulse(std::numeric_limits<uint64_t>::max(), 1, 1500)) return 5;
  return 0;
}
