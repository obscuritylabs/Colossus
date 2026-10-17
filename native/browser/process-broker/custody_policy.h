#ifndef COLOSSUS_BROWSER_CUSTODY_POLICY_H_
#define COLOSSUS_BROWSER_CUSTODY_POLICY_H_
#include <stddef.h>
#ifdef __cplusplus
extern "C" {
#endif
/* Trusted pinned source only. Clamp every positive file grant to exclude the
 * canonical custody root, including more-specific stock grants and extension
 * filters. Remove explicit positive file-extension issuance and clamp all
 * positive Mach lookup filters to exclude SecurityServer/securityd/trustd
 * global/XPC names. Generic file* still grants its excluded-root file operations.
 * Unknown mixed grants,
 * malformed source and unsafe/noncanonical roots fail closed. This constructs
 * policy; native per-role denial acceptance still must prove its application.
 * Caller owns the bounded (<=128 KiB) result through free(). */
int colossus_owned_custody_policy(const char* pinned,size_t size,
    const char* custody_root,char** output);
#ifdef __cplusplus
}
#endif
#endif
