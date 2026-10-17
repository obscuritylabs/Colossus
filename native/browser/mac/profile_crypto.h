#ifndef COLOSSUS_MAC_PROFILE_CRYPTO_H_
#define COLOSSUS_MAC_PROFILE_CRYPTO_H_
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

// Developer-only, temporary profile-data encryption. Neither PKI custody nor a
// hostile-process boundary. Unsupported dyld/Security SPIs never enable release.
enum colossus_mac_profile_crypto_status {
  COLOSSUS_MAC_PROFILE_CRYPTO_READY = 0,
  COLOSSUS_MAC_PROFILE_CRYPTO_DENIED = 1,
  COLOSSUS_MAC_PROFILE_CRYPTO_UNAVAILABLE = 2,
  COLOSSUS_MAC_PROFILE_CRYPTO_OUTCOME_UNKNOWN = 3,
};

// Closed Debug diagnostics. Categories contain no paths, native errors, code
// hashes, item attributes or secret bytes; they grant no additional operation.
enum colossus_mac_profile_crypto_phase {
  COLOSSUS_MAC_PROFILE_PHASE_NOT_ATTEMPTED = 0,
  COLOSSUS_MAC_PROFILE_PHASE_ROUTING = 1,
  COLOSSUS_MAC_PROFILE_PHASE_PREPARE_GUARD = 2,
  COLOSSUS_MAC_PROFILE_PHASE_SECURITY_SYMBOLS = 3,
  COLOSSUS_MAC_PROFILE_PHASE_PARENT_PATH = 4,
  COLOSSUS_MAC_PROFILE_PHASE_PARENT_OPEN = 5,
  COLOSSUS_MAC_PROFILE_PHASE_PARENT_SECURITY = 6,
  COLOSSUS_MAC_PROFILE_PHASE_INTERACTION_POLICY = 7,
  COLOSSUS_MAC_PROFILE_PHASE_CODE_SELF = 8,
  COLOSSUS_MAC_PROFILE_PHASE_CODE_STATIC = 9,
  COLOSSUS_MAC_PROFILE_PHASE_CODE_DESIGNATED_REQUIREMENT = 10,
  COLOSSUS_MAC_PROFILE_PHASE_CODE_DESIGNATED_VALIDITY = 11,
  COLOSSUS_MAC_PROFILE_PHASE_CODE_SIGNING_INFORMATION = 12,
  COLOSSUS_MAC_PROFILE_PHASE_CODE_SIGNATURE_PROPERTIES = 13,
  COLOSSUS_MAC_PROFILE_PHASE_CODE_EXACT_REQUIREMENT = 14,
  COLOSSUS_MAC_PROFILE_PHASE_CODE_EXACT_VALIDITY = 15,
  COLOSSUS_MAC_PROFILE_PHASE_OWNED_DIRECTORY = 16,
  COLOSSUS_MAC_PROFILE_PHASE_OWNED_STORE = 17,
  COLOSSUS_MAC_PROFILE_PHASE_OWNED_ITEM = 18,
  COLOSSUS_MAC_PROFILE_PHASE_OWNED_ITEM_ACCESS = 19,
  COLOSSUS_MAC_PROFILE_PHASE_READY = 20,
};

// The trusted native host supplies a canonical, private, freshly owned parent,
// and its OWN imported function addresses, in this exact order: CopyMatching,
// Add, Update, Delete. All four must match the launch dependency's interposers
// before any Keychain operation. No path/secret arrives from renderer/model IPC.
__attribute__((visibility("default"))) int32_t colossus_mac_profile_crypto_prepare(
    const char* canonical_owned_parent, const void* const imported_functions[4]);
__attribute__((visibility("default"))) int32_t colossus_mac_profile_crypto_valid(void);
__attribute__((visibility("default"))) int32_t colossus_mac_profile_crypto_prepare_phase(void);
// Closed operator fixture only: initial creation/removal, no generic-password
// item or browser capability. Never used by the ordinary native host bootstrap.
__attribute__((visibility("default"))) int32_t colossus_mac_profile_crypto_prepare_store_fixture(
    const char* canonical_owned_parent, const void* const imported_functions[4]);
__attribute__((visibility("default"))) int32_t colossus_mac_profile_crypto_store_fixture_valid(void);
// Revoke first, drain lookups, then retire exact owned artifacts. Unknown
// identity or cleanup remains an obligation; never delete a replacement tree.
__attribute__((visibility("default"))) int32_t colossus_mac_profile_crypto_finish(void);

#ifdef __cplusplus
}
#endif
#endif
