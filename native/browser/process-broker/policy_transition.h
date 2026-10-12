#ifndef COLOSSUS_OWNED_PROCESS_POLICY_TRANSITION_H_
#define COLOSSUS_OWNED_PROCESS_POLICY_TRANSITION_H_
#include <stddef.h>
#ifdef __cplusplus
extern "C" {
#endif
/* Transform only trusted pinned builder output. Browser input never supplies
 * policy text. Remove every positive process creation/exec expression before
 * appending final denies; job-creation/mach-register grants are removed too,
 * because a generic deny cannot override a more specific
 * stock allow. Unknown/mixed process grants and malformed source fail closed.
 * Returned source remains <=128 KiB and is owned by the caller via free(). */
int colossus_owned_process_policy(const char* pinned,size_t size,char** output);
#ifdef __cplusplus
}
#endif
#endif
