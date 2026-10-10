#ifndef COLOSSUS_CEF_CUSTODY_PROC_LINUX_H
#define COLOSSUS_CEF_CUSTODY_PROC_LINUX_H
// Private to the OFF-default native custody experiment; never a host ABI.
#include <cstdint>
#include <string>
#include <sys/types.h>
namespace colossus::custody_test {
struct BrowserIdentity {
  pid_t pid = 0, group = 0;
  uid_t uid = 0;
  uint64_t start = 0;
  dev_t device = 0;
  ino_t inode = 0;
  unsigned filters = 0;
};
struct ProcessStat {
  pid_t pid = 0, parent = 0, group = 0;
  uint64_t start = 0;
};
struct ProcessStatus {
  pid_t pid = 0, namespace_pid = 0;
  uid_t uid = 0;
  unsigned seccomp = 0, filters = 0;
};
enum class Attestation {
  Verified, InvalidBinding, ProcUnavailable, NotFound, Ambiguous,
  SandboxUnverified, MetadataUnavailable, PidfdUnavailable, Unstable
};
// Bounded proc parsers are exposed only inside this internal test namespace.
bool ParseStat(const std::string& bytes, ProcessStat* result);
bool ParseStatus(const std::string& bytes, ProcessStatus* result);
bool Capture(BrowserIdentity* result);
// Requires one unambiguous live descendant, exact executable/group/challenge,
// two stable kernel snapshots and an additional inner seccomp filter.
bool Attest(const BrowserIdentity& browser, pid_t namespace_pid,
            const std::string& nonce, const std::string& fixture_url);
Attestation AttestDetailed(const BrowserIdentity& browser, pid_t namespace_pid,
                          const std::string& nonce, const std::string& fixture_url);
}
#endif
