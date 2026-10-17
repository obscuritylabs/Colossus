#ifndef COLOSSUS_CEF_SYNTHETIC_PROFILE_H_
#define COLOSSUS_CEF_SYNTHETIC_PROFILE_H_

#include <stdint.h>

namespace colossus {
// Closed pre-bootstrap selection only. The native-only preparation ABI is
// unavailable in normal builds; this does not select any real Keychain store.
const char* SyntheticProfileSwitch(bool browser_process);
int32_t SyntheticProfileBootstrap(int32_t mode);
bool SyntheticProfileAllowsPersistence();
}

#endif
