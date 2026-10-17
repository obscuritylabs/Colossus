// Standalone tests for the OFF-only proc attestor. They do not attest Chromium
// or private-key custody: the child installs an allow-all extra BPF filter.
#include "../src/custody_proc_linux.h"
#include <array>
#include <cassert>
#include <cerrno>
#include <csignal>
#include <cstdlib>
#include <cstring>
#include <fcntl.h>
#include <linux/filter.h>
#include <linux/seccomp.h>
#include <poll.h>
#include <sys/prctl.h>
#include <sys/wait.h>
#include <unistd.h>

namespace {
using namespace colossus::custody_test;
constexpr char kNonce[] = "0123456789abcdef0123456789abcdef";
constexpr char kUrl[] = "https://127.0.0.1:9443/client-check";
std::string Status(const std::string& extra = "") {
  return "Name:\tfixture\nPid:\t42\nUid:\t1000\t1000\t1000\t1000\n"
    "NSpid:\t42\t2\nSeccomp:\t2\nSeccomp_filters:\t2\n" + extra;
}
void ParseTests() {
  ProcessStat stat{}; ProcessStatus status{};
  const std::string fields = " R 7 8 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 123";
  assert(ParseStat("42 (parenthesis ) process)" + fields, &stat));
  assert(stat.pid == 42 && stat.parent == 7 && stat.group == 8 && stat.start == 123);
  assert(!ParseStat("42 (name) R -1 8", &stat));
  assert(!ParseStat("2147483648 (name)" + fields, &stat));
  assert(!ParseStat("42 (name)" + fields + std::string(16384, 'x'), &stat));
  assert(ParseStatus(Status(), &status));
  assert(status.pid == 42 && status.namespace_pid == 2 && status.uid == 1000 && status.filters == 2);
  assert(!ParseStatus(Status("Seccomp:\t2\n"), &status));
  assert(!ParseStatus(Status("NSpid:\t42\t2\n"), &status));
  assert(!ParseStatus("Pid:\t42\nUid:\t1000\t0\t1000\t1000\nNSpid:\t42\nSeccomp:\t2\nSeccomp_filters:\t2\n", &status));
  assert(!ParseStatus("Pid:\t42\nUid:\t1000\t1000\t1000\t1000\nNSpid:\t43\nSeccomp:\t2\nSeccomp_filters:\t2\n", &status));
  assert(!ParseStatus("Pid:\t42\nUid:\t1000\t1000\t1000\t1000\nNSpid:\t42\nSeccomp:\t2\nSeccomp_filters:\t0\n", &status));
  assert(!ParseStatus(Status() + std::string(16384, 'x'), &status));
}
int Child(int argc, char** argv) {
  sock_filter instructions[] = { BPF_STMT(BPF_RET | BPF_K, SECCOMP_RET_ALLOW) };
  sock_fprog program{1, instructions};
  if (prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) || prctl(PR_SET_SECCOMP, SECCOMP_MODE_FILTER, &program)) return 2;
  if (argc > 4 && std::strcmp(argv[4], "--foreign-group") == 0 && setpgid(0, 0)) return 6;
  if (argc > 4 && std::strcmp(argv[4], "--nondumpable") == 0 && prctl(PR_SET_DUMPABLE, 0)) return 7;
  const auto pid = getpid();
  if (write(3, &pid, sizeof(pid)) != sizeof(pid)) return 3;
  while (true) pause();
}
pid_t Launch(const char* executable, const char* extra = nullptr) {
  std::array<int, 2> descriptors{};
  assert(pipe2(descriptors.data(), O_CLOEXEC) == 0);
  const pid_t parent = getpid(), pid = fork(); assert(pid >= 0);
  if (pid == 0) {
    // A failed assertion in the test parent must not leave a paused helper.
    if (prctl(PR_SET_PDEATHSIG, SIGKILL) || getppid() != parent) _exit(8);
    close(descriptors[0]);
    if (descriptors[1] != 3) { if (dup2(descriptors[1], 3) < 0) _exit(4); close(descriptors[1]); }
    else if (fcntl(3, F_SETFD, 0)) _exit(4);
    const std::string nonce = std::string("--colossus-native-nss-open-test=") + kNonce;
    const std::string fixture = std::string("--colossus-native-nss-open-fixture=") + kUrl;
    if (extra) execl(executable, executable, "--type=renderer", nonce.c_str(), fixture.c_str(), extra, nullptr);
    else execl(executable, executable, "--type=renderer", nonce.c_str(), fixture.c_str(), nullptr);
    _exit(5);
  }
  close(descriptors[1]);
  pollfd ready{descriptors[0], POLLIN, 0};
  assert(poll(&ready, 1, 5000) == 1 && (ready.revents & POLLIN));
  pid_t reported = 0;
  assert(read(descriptors[0], &reported, sizeof(reported)) == sizeof(reported) && reported == pid);
  close(descriptors[0]); return pid;
}
void Stop(pid_t pid) {
  assert(kill(pid, SIGTERM) == 0);
  int status = 0; assert(waitpid(pid, &status, 0) == pid && WIFSIGNALED(status));
}
}
int main(int argc, char** argv) {
  if (argc > 1 && std::strcmp(argv[1], "--type=renderer") == 0) return Child(argc, argv);
  ParseTests();
  colossus::custody_test::BrowserIdentity browser;
  assert(colossus::custody_test::Capture(&browser));
  const auto child = Launch(argv[0]);
  assert(colossus::custody_test::Attest(browser, child, kNonce, kUrl));
  assert(colossus::custody_test::AttestDetailed(browser, child, kNonce, kUrl) == Attestation::Verified);
  assert(!colossus::custody_test::Attest(browser, child, "ffffffffffffffffffffffffffffffff", kUrl));
  assert(!colossus::custody_test::Attest(browser, child, kNonce, "https://127.0.0.1:9444/client-check"));
  auto foreign = browser; ++foreign.inode;
  assert(!colossus::custody_test::Attest(foreign, child, kNonce, kUrl));
  foreign = browser; ++foreign.start;
  assert(!colossus::custody_test::Attest(foreign, child, kNonce, kUrl));
  Stop(child);
  assert(!colossus::custody_test::Attest(browser, child, kNonce, kUrl));
  const auto duplicated = Launch(argv[0], "--colossus-native-nss-open-test=0123456789abcdef0123456789abcdef");
  assert(!colossus::custody_test::Attest(browser, duplicated, kNonce, kUrl));
  Stop(duplicated);
  const auto foreign_group = Launch(argv[0], "--foreign-group");
  assert(!colossus::custody_test::Attest(browser, foreign_group, kNonce, kUrl));
  Stop(foreign_group);
  const auto inaccessible = Launch(argv[0], "--nondumpable");
  assert(colossus::custody_test::AttestDetailed(browser, inaccessible, kNonce, kUrl) == Attestation::MetadataUnavailable);
  Stop(inaccessible);
  return 0;
}
