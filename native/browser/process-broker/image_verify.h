#ifndef COLOSSUS_OWNED_PROCESS_IMAGE_VERIFY_H_
#define COLOSSUS_OWNED_PROCESS_IMAGE_VERIFY_H_
#include <stdint.h>
/* Fixed-image constructor only: path/hash come from the creator's independently
 * verified manifest. Browser arguments never nominate a library or digest.
 * Verify a canonical private regular one-link library, bounded SHA256 stream,
 * stable object metadata and no writable group/other aliases before dlopen.
 * Native packaging separately verifies signing and library-validation policy. */
int colossus_owned_library_verify(const char* path,const uint8_t expected_sha256[32]);
#endif
