#ifndef COLOSSUS_MAC_CUSTODY_BOUNDARY_H_
#define COLOSSUS_MAC_CUSTODY_BOUNDARY_H_
#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>
#ifdef __cplusplus
extern "C" {
#endif
// Narrow locally linked platform boundary. No process launch, audit session
// joining, Keychain call, trust-store mutation or generic policy interface.
uint32_t colossus_custody_parent_pid(void);
bool colossus_custody_executable(char*, size_t);
bool colossus_custody_code(const char*, const uint8_t*);
void* colossus_custody_create(const char*, const uint8_t*);
int colossus_custody_parent_fd(void*);
bool colossus_custody_begin(void*, const uint8_t*, int, int, uint64_t, uint32_t);
bool colossus_custody_exclusive(void*, uint64_t);
bool colossus_custody_seal(void*, uint64_t, const int*, size_t);
bool colossus_custody_checkpoint(void*, uint64_t, uint32_t);
bool colossus_custody_retire_unchanged(void*, uint64_t, const int*, size_t);
uint32_t colossus_custody_descriptor_kind(void*, int);
bool colossus_custody_retired_suffix(void*, uint8_t*);
void colossus_custody_end(void*, uint64_t);
void colossus_custody_drop(void*);
#ifdef __cplusplus
}
#endif
#endif
