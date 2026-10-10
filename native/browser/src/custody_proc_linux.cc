#include "custody_proc_linux.h"
#include <array>
#include <cerrno>
#include <charconv>
#include <chrono>
#include <climits>
#include <cstring>
#include <dirent.h>
#include <fcntl.h>
#include <linux/magic.h>
#include <poll.h>
#include <sstream>
#include <string_view>
#include <sys/stat.h>
#include <sys/statfs.h>
#include <sys/syscall.h>
#include <unistd.h>
#include <vector>

namespace colossus::custody_test {
namespace {
constexpr size_t kBound = 16384;
class Descriptor {
 public:
  explicit Descriptor(int value = -1) : value_(value) {}
  ~Descriptor() { if (value_ >= 0) close(value_); }
  Descriptor(const Descriptor&) = delete;
  Descriptor& operator=(const Descriptor&) = delete;
  int get() const { return value_; }
  int release() { const int value = value_; value_ = -1; return value; }
 private:
  int value_;
};
bool Number(std::string_view text, uint64_t* value) {
  if (text.empty() || text.size() > 20) return false;
  const auto parsed = std::from_chars(text.data(), text.data() + text.size(), *value);
  return parsed.ec == std::errc{} && parsed.ptr == text.data() + text.size();
}
bool ProcessNumber(std::string_view text, pid_t* value) {
  uint64_t number = 0;
  if (!Number(text, &number) || number > INT_MAX) return false;
  *value = static_cast<pid_t>(number); return true;
}
std::vector<std::string> Fields(const std::string& text) {
  std::istringstream stream(text); std::vector<std::string> result;
  std::string value;
  while (stream >> value) result.push_back(value);
  return result;
}
bool Read(int directory, const char* leaf, std::string* result) {
  Descriptor file(openat(directory, leaf, O_RDONLY | O_NOFOLLOW | O_CLOEXEC));
  struct stat metadata{};
  if (file.get() < 0 || fstat(file.get(), &metadata) || !S_ISREG(metadata.st_mode)) return false;
  result->clear(); std::array<char, 1024> bytes{};
  while (result->size() <= kBound) {
    const auto count = read(file.get(), bytes.data(), bytes.size());
    if (count < 0) return false;
    if (count == 0) return result->size() <= kBound;
    result->append(bytes.data(), static_cast<size_t>(count));
  }
  return false;
}
int Proc() {
  const int file = open("/proc", O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
  struct statfs type{};
  if (file < 0) return -1;
  if (fstatfs(file, &type) || type.f_type != PROC_SUPER_MAGIC) { close(file); return -1; }
  return file;
}
int Directory(int root, pid_t pid) {
  return openat(root, std::to_string(pid).c_str(), O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
}
bool Executable(int directory, dev_t device, ino_t inode) {
  // This is the kernel-owned procfs exe link, intentionally followed. An
  // inaccessible non-dumpable renderer fails closed; no ptrace override.
  Descriptor file(openat(directory, "exe", O_PATH | O_CLOEXEC));
  struct stat metadata{};
  return file.get() >= 0 && fstat(file.get(), &metadata) == 0 &&
    S_ISREG(metadata.st_mode) && metadata.st_dev == device && metadata.st_ino == inode;
}
bool Command(int directory, const std::string& nonce, const std::string& url) {
  std::string bytes;
  if (!Read(directory, "cmdline", &bytes) || bytes.empty() || bytes.back() != '\0') return false;
  unsigned type = 0, challenge = 0, fixture = 0;
  for (size_t cursor = 0; cursor < bytes.size();) {
    const auto end = bytes.find('\0', cursor);
    if (end == std::string::npos) return false;
    const auto argument = bytes.substr(cursor, end - cursor);
    if (argument.starts_with("--type=")) { if (argument != "--type=renderer") return false; ++type; }
    if (argument.starts_with("--colossus-native-nss-open-test=")) {
      if (argument != "--colossus-native-nss-open-test=" + nonce) return false;
      ++challenge;
    }
    if (argument.starts_with("--colossus-native-nss-open-fixture=")) {
      if (argument != "--colossus-native-nss-open-fixture=" + url) return false;
      ++fixture;
    }
    cursor = end + 1;
  }
  return type == 1 && challenge == 1 && fixture == 1;
}
bool Snapshot(int directory, ProcessStat* stat, ProcessStatus* status) {
  std::string first, second;
  return Read(directory, "stat", &first) && Read(directory, "status", &second) &&
    ParseStat(first, stat) && ParseStatus(second, status) && stat->pid == status->pid;
}
bool Same(const ProcessStat& a, const ProcessStat& b,
          const ProcessStatus& c, const ProcessStatus& d) {
  return a.pid == b.pid && a.parent == b.parent && a.group == b.group && a.start == b.start &&
    c.pid == d.pid && c.namespace_pid == d.namespace_pid && c.uid == d.uid &&
    c.seccomp == d.seccomp && c.filters == d.filters;
}
bool Alive(int descriptor) {
  pollfd value{descriptor, POLLIN, 0};
  return poll(&value, 1, 0) == 0;
}
bool Descendant(int root, const BrowserIdentity& browser, ProcessStat child) {
  for (unsigned depth = 0; depth < 16; ++depth) {
    if (child.parent <= 0 || child.parent == child.pid) return false;
    Descriptor directory(Directory(root, child.parent));
    std::string bytes; ProcessStat parent{};
    if (directory.get() < 0 || !Read(directory.get(), "stat", &bytes) || !ParseStat(bytes, &parent) ||
        parent.pid != child.parent || parent.group != browser.group || parent.start > child.start)
      return false;
    if (parent.pid == browser.pid) return parent.start == browser.start;
    child = parent;
  }
  return false;
}
int Candidate(int root, const BrowserIdentity& browser, pid_t pid, pid_t namespace_pid,
               const std::string& nonce, const std::string& url,
               bool* anchored, Attestation* failure) {
  *anchored = false;
  Descriptor directory(Directory(root, pid));
  ProcessStat first{}, second{}; ProcessStatus status{}, after{};
  if (directory.get() < 0 || !Snapshot(directory.get(), &first, &status) || first.pid != pid ||
      first.group != browser.group || first.start < browser.start || status.uid != browser.uid ||
      status.namespace_pid != namespace_pid || !Command(directory.get(), nonce, url))
    return -1;
  *anchored = true;
  if (status.seccomp != 2 || status.filters <= browser.filters) {
    *failure = Attestation::SandboxUnverified; return -1;
  }
  if (!Executable(directory.get(), browser.device, browser.inode)) {
    *failure = Attestation::MetadataUnavailable; return -1;
  }
  Descriptor pidfd(static_cast<int>(syscall(SYS_pidfd_open, pid, 0)));
  if (pidfd.get() < 0) { *failure = Attestation::PidfdUnavailable; return -1; }
  struct stat retained{}, current{};
  Descriptor current_directory(Directory(root, pid));
  const bool valid = pidfd.get() >= 0 && Alive(pidfd.get()) && Descendant(root, browser, first) &&
    fstat(directory.get(), &retained) == 0 && current_directory.get() >= 0 &&
    fstat(current_directory.get(), &current) == 0 && retained.st_dev == current.st_dev &&
    retained.st_ino == current.st_ino && Snapshot(directory.get(), &second, &after) &&
    Same(first, second, status, after) && Command(directory.get(), nonce, url) &&
    Executable(directory.get(), browser.device, browser.inode) && Alive(pidfd.get());
  if (!valid) *failure = Attestation::Unstable;
  return valid ? pidfd.release() : -1;
}
}

bool ParseStat(const std::string& bytes, ProcessStat* result) {
  if (!result || bytes.size() > kBound) return false;
  const auto begin = bytes.find(" ("), end = bytes.rfind(") ");
  if (begin == std::string::npos || end == std::string::npos || end <= begin) return false;
  const auto fields = Fields(bytes.substr(end + 2)); ProcessStat value{};
  if (fields.size() < 20 || fields[0].size() != 1 ||
      !ProcessNumber(std::string_view(bytes).substr(0, begin), &value.pid) || value.pid <= 0 ||
      !ProcessNumber(fields[1], &value.parent) || !ProcessNumber(fields[2], &value.group) ||
      value.group <= 0 || !Number(fields[19], &value.start) || value.start == 0) return false;
  *result = value; return true;
}
bool ParseStatus(const std::string& bytes, ProcessStatus* result) {
  if (!result || bytes.size() > kBound) return false;
  ProcessStatus value{}; unsigned pid = 0, uid = 0, ns = 0, seccomp = 0, filters = 0;
  std::istringstream lines(bytes); std::string line;
  while (std::getline(lines, line)) {
    const auto separator = line.find(':');
    if (separator == std::string::npos) return false;
    const auto name = line.substr(0, separator);
    const auto fields = Fields(line.substr(separator + 1));
    uint64_t number = 0;
    if (name == "Pid") {
      if (++pid != 1 || fields.size() != 1 || !ProcessNumber(fields[0], &value.pid) || value.pid <= 0) return false;
    } else if (name == "Uid") {
      if (++uid != 1 || fields.size() != 4 || !Number(fields[0], &number) || number > UINT_MAX) return false;
      for (const auto& field : fields) if (field != fields[0]) return false;
      value.uid = static_cast<uid_t>(number);
    } else if (name == "NSpid") {
      if (++ns != 1 || fields.empty() || fields.size() > 16) return false;
      for (const auto& field : fields) if (!ProcessNumber(field, &value.namespace_pid) || value.namespace_pid <= 0) return false;
      if (fields.front() != std::to_string(value.pid)) return false;
    } else if (name == "Seccomp" || name == "Seccomp_filters") {
      if (fields.size() != 1 || !Number(fields[0], &number) || number > UINT_MAX) return false;
      if (name == "Seccomp") { if (++seccomp != 1) return false; value.seccomp = static_cast<unsigned>(number); }
      else { if (++filters != 1) return false; value.filters = static_cast<unsigned>(number); }
    }
  }
  if (pid != 1 || uid != 1 || ns != 1 || seccomp != 1 || filters != 1 || value.seccomp > 2 ||
      (value.seccomp == 0 && value.filters != 0) || (value.seccomp == 2 && value.filters == 0)) return false;
  *result = value; return true;
}
bool Capture(BrowserIdentity* result) {
  if (!result) return false;
  Descriptor root(Proc()); Descriptor directory(root.get() < 0 ? -1 : Directory(root.get(), getpid()));
  // The kernel-owned self link also proves this procfs view names our current
  // PID namespace; a mounted ancestor namespace must not bind a coincident PID.
  Descriptor self(root.get() < 0 ? -1 : openat(root.get(), "self", O_RDONLY | O_DIRECTORY | O_CLOEXEC));
  ProcessStat stat{}; ProcessStatus status{};
  Descriptor executable(directory.get() < 0 ? -1 : openat(directory.get(), "exe", O_PATH | O_CLOEXEC));
  struct stat metadata{}, named{}, own{};
  if (directory.get() < 0 || self.get() < 0 || fstat(directory.get(), &named) || fstat(self.get(), &own) ||
      named.st_dev != own.st_dev || named.st_ino != own.st_ino ||
      !Snapshot(self.get(), &stat, &status) || stat.pid != getpid() ||
      stat.group != getpgrp() || status.uid != geteuid() || executable.get() < 0 ||
      fstat(executable.get(), &metadata) || !S_ISREG(metadata.st_mode)) return false;
  *result = {stat.pid, stat.group, status.uid, stat.start, metadata.st_dev, metadata.st_ino, status.filters};
  return true;
}
Attestation AttestDetailed(const BrowserIdentity& browser, pid_t namespace_pid,
                          const std::string& nonce, const std::string& fixture_url) {
  if (browser.pid != getpid() || browser.group != getpgrp() || browser.uid != geteuid() ||
      namespace_pid <= 0 || nonce.size() != 32 || fixture_url.size() > 4096) return Attestation::InvalidBinding;
  Descriptor root(Proc());
  DIR* entries = root.get() < 0 ? nullptr : fdopendir(dup(root.get()));
  if (!entries) return Attestation::ProcUnavailable;
  unsigned count = 0, potential = 0;
  Attestation failure = Attestation::NotFound;
  bool complete = true;
  int accepted = -1; pid_t accepted_pid = 0;
  const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(2);
  errno = 0;
  while (const auto* entry = readdir(entries)) {
    if (++count > 4096 || std::chrono::steady_clock::now() > deadline) { complete = false; break; }
    pid_t pid = 0;
    if (ProcessNumber(entry->d_name, &pid) && pid > 0) {
      bool anchored = false;
      const int candidate = Candidate(root.get(), browser, pid, namespace_pid, nonce, fixture_url, &anchored, &failure);
      if (anchored) ++potential;
      if (candidate >= 0) {
        if (accepted < 0) { accepted = candidate; accepted_pid = pid; }
        else close(candidate);
      }
    }
    if (potential > 1) break;
    // Candidate process disappearance is ordinary; only readdir's own errno
    // describes a directory scan failure, so reset it before the next call.
    errno = 0;
  }
  const bool scan_valid = complete && errno == 0;
  closedir(entries);
  Descriptor retained(accepted);
  if (!scan_valid) return Attestation::ProcUnavailable;
  if (potential > 1) return Attestation::Ambiguous;
  if (accepted < 0) return failure;
  bool anchored = false;
  Descriptor repeated(Candidate(root.get(), browser, accepted_pid, namespace_pid, nonce, fixture_url, &anchored, &failure));
  return anchored && repeated.get() >= 0 && Alive(retained.get()) && Alive(repeated.get()) ?
    Attestation::Verified : Attestation::Unstable;
}
bool Attest(const BrowserIdentity& browser, pid_t namespace_pid,
            const std::string& nonce, const std::string& fixture_url) {
  return AttestDetailed(browser, namespace_pid, nonce, fixture_url) == Attestation::Verified;
}
}
