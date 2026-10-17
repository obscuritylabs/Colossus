#ifndef COLOSSUS_OWNED_CUSTODY_PROBE_H_
#define COLOSSUS_OWNED_CUSTODY_PROBE_H_
#include <stdint.h>
#include <limits.h>
struct colossus_custody_probe_binding {
  char root[PATH_MAX];char profile[PATH_MAX];uint8_t policy_sha256[32];
};
/* Fixed signed policy resource only; never an argv/env/browser path. */
int colossus_owned_custody_probe_binding(struct colossus_custody_probe_binding*);
/* Emit exact pre-body native evidence and consume one bounded creator ACK.
 * Mode1 is a private Debug operator probe-only exit; mode0 admits body loading.
 * A mode1 read-extension cookie can refer only to the disposable fixed marker.
 * No receipt is emitted after any denial fails. Return 1(normal),2(probe-only),0. */
int colossus_owned_custody_probe(int fd,const struct colossus_custody_probe_binding*,
    const uint8_t nonce[16],uint8_t role,const uint8_t body_sha256[32],
    uint8_t mode,const char* read_extension);
#endif
