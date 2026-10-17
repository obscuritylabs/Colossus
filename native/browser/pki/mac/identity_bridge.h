#ifndef COLOSSUS_MAC_IDENTITY_BRIDGE_H_
#define COLOSSUS_MAC_IDENTITY_BRIDGE_H_
#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>
#ifdef __cplusplus
extern "C" {
#endif
typedef struct { const uint8_t* data; size_t size; } ColossusPublicBytes;
typedef struct {
  void* context;
  bool (*begin)(void*, const uint8_t*, int, int, uint64_t, uint32_t);
  bool (*exclusive)(void*, uint64_t);
  bool (*seal)(void*, uint64_t, const int*, size_t);
  void (*end)(void*, uint64_t);
  bool (*signing_admitted)(void*, const uint8_t*);
  bool (*checkpoint)(void*, uint64_t, uint32_t);
  bool (*retire_unchanged)(void*, uint64_t, const int*, size_t);
} ColossusMutationCallbacks;
typedef struct {
  int parent_fd;
  ColossusPublicBytes parent_path, origin, reviewed_leaf, encrypted_pkcs12;
  const ColossusPublicBytes* ca;
  size_t ca_count;
  const uint8_t* code_hash;
  const uint8_t* generation;
  uint8_t* password;
  size_t password_size;
  ColossusMutationCallbacks mutations;
} ColossusIdentityEnrollment;
// Private locally linked C ABI. These opaque process-local pointers never cross
// native bootstrap or browser/Core IPC, and expose no private key/export method.
uint32_t colossus_mac_identity_prepare(const ColossusIdentityEnrollment*, void**);
uint32_t colossus_mac_identity_begin(void*, const uint8_t*, ColossusPublicBytes, const uint8_t*, uint64_t, uint64_t*);
uint32_t colossus_mac_identity_verify(void*, uint64_t, const uint8_t*, uint32_t, const uint8_t*, uint8_t*, size_t*);
void colossus_mac_identity_cancel(void*, uint64_t, const uint8_t*);
void colossus_mac_identity_revoke(void*);
uint32_t colossus_mac_identity_finish(void*);
bool colossus_mac_identity_diagnostic(void*, uint32_t*, uint32_t*, int32_t*);
void colossus_mac_identity_drop(void*);
// Acceptance only: digest default/search-list metadata without changing it.
bool colossus_mac_keychain_metadata(uint8_t*);
bool colossus_mac_validate_ca(const uint8_t*, size_t);
bool colossus_mac_validate_identity(const uint8_t*, size_t);
bool colossus_mac_validate_origin(const uint8_t*, size_t);
#ifdef __cplusplus
}
#endif
#endif
