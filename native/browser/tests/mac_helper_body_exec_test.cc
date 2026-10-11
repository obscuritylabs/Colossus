// Exercise the exact role policy before dyld/libSystem initializes a fresh Body.
// This toy links the same SDK system frameworks as a CEF helper but never loads
// CEF, initializes AppKit, queries Keychain, or supplies an audit-session port.
#include "network_envelope.h"
#include <arpa/inet.h>
#include <netinet/in.h>
#include <sys/socket.h>
#include <cerrno>
#include <cstdio>
#include <cstring>
#include <fcntl.h>
#include <filesystem>
#include <limits.h>
#include <mach-o/dyld.h>
#include <signal.h>
#include <string>
#include <sys/stat.h>
#include <sys/sysctl.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

extern "C" int sandbox_init_with_parameters(const char*, unsigned long long,
                                             const char* const*, char**);
extern "C" void sandbox_free_error(char*);
extern "C" int sandbox_check(pid_t, const char*, int, ...);
namespace fs = std::filesystem;

namespace {
int Body(const char* marker, const char* home_marker) {
  // The std::string also requires the same system C++ runtime as the real Body.
  const std::string label("helper fixture");
  if (label.empty() || sandbox_check(getpid(), nullptr, 0) != 1) return 31;
  int fd = open(marker, O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
  if (fd >= 0) { close(fd); return 32; }
  if (errno != EPERM && errno != EACCES) return 33;
  fd = open(home_marker, O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
  if (fd >= 0) { close(fd); return 34; }
  if (errno != EPERM && errno != EACCES) return 35;
  int network = socket(AF_INET, SOCK_STREAM, 0);
  if (network < 0) return 36;
  sockaddr_in address{.sin_len = sizeof(address), .sin_family = AF_INET,
      .sin_port = htons(1), .sin_addr = {.s_addr = htonl(INADDR_LOOPBACK)}};
  int connected = connect(network, reinterpret_cast<sockaddr*>(&address), sizeof(address));
  int saved = errno;
  close(network);
  if (connected != -1 || (saved != EPERM && saved != EACCES)) return 37;
  std::puts("PASS toy helper Body main; broker/home fixture data and unapproved inet outbound denied; exact root-directory FD grant; descendant file data remains denied");
  return 0;
}

uint32_t OSVersion() {
  char value[64]; size_t size = sizeof(value);
  if (sysctlbyname("kern.osproductversion", value, &size, nullptr, 0) ||
      !size || size > sizeof(value)) return 0;
  value[sizeof(value) - 1] = 0;
  unsigned major = 0, minor = 0;
  if (std::sscanf(value, "%u.%u", &major, &minor) < 1 ||
      major < 14 || major > 99 || minor > 99) return 0;
  return major * 100 + minor;
}

bool WaitOwned(pid_t child, int role) {
  int status = 0;
  for (int pass = 0; pass < 600; ++pass) {
    pid_t reaped = waitpid(child, &status, WNOHANG);
    if (reaped == child) {
      std::fprintf(stderr, "toy helper result pid=%d role=%d exited=%d status=%d signal=%d\n",
        child, role, WIFEXITED(status), WIFEXITED(status) ? WEXITSTATUS(status) : -1,
        WIFSIGNALED(status) ? WTERMSIG(status) : 0);
      return WIFEXITED(status) && WEXITSTATUS(status) == 0;
    }
    if (reaped < 0 && errno != EINTR) return false;
    timespec delay{0, 10000000};
    nanosleep(&delay, nullptr);
  }
  kill(child, SIGKILL);
  while (waitpid(child, &status, 0) < 0 && errno == EINTR) {}
  return false;
}

bool Run(const colossus_mac_network_binding& binding, const std::string& image,
         const std::string& marker, const std::string& home_marker, colossus_mac_network_role role,
         const char* suffix, uint32_t version) {
  const std::string name = std::string("Colossus Browser Helper") + suffix;
  const fs::path macos = fs::path(binding.bundle_path) / "Contents/Frameworks" /
      (name + ".app") / "Contents/MacOS";
  std::error_code error;
  fs::create_directories(macos, error);
  if (error) return false;
  const std::string front = (macos / name).string();
  const std::string body = front + " Body";
  if (!fs::copy_file(image, body, fs::copy_options::none, error) || error ||
      chmod(body.c_str(), 0700)) return false;
  char* source = nullptr;
  if (!colossus_mac_helper_body_policy(role, &binding, front.c_str(), body.c_str(),
                                       getpid(), version, &source)) return false;
  pid_t child = fork();
  if (child == 0) {
    alarm(5);
    char* message = nullptr;
    const char* parameters[] = {nullptr};
    int applied = sandbox_init_with_parameters(source, 0, parameters, &message);
    if (message) sandbox_free_error(message);
    if (applied || sandbox_check(getpid(), nullptr, 0) != 1) _exit(20);
    char* arguments[] = {const_cast<char*>(body.c_str()),
      const_cast<char*>("--owned-body"), const_cast<char*>(marker.c_str()), const_cast<char*>(home_marker.c_str()), nullptr};
    char* environment[] = {nullptr};
    execve(body.c_str(), arguments, environment);
    _exit(21);
  }
  free(source);
  return child > 0 && WaitOwned(child, role);
}
}  // namespace

int main(int argc, char** argv) {
  if (argc == 4 && std::strcmp(argv[1], "--owned-body") == 0) return Body(argv[2], argv[3]);
  if (argc != 1) return 30;
  umask(0077);
  char raw[PATH_MAX], image[PATH_MAX]; uint32_t size = sizeof(raw);
  if (_NSGetExecutablePath(raw, &size) || !realpath(raw, image)) return 1;
  uint32_t version = OSVersion();
  if (!version) return 1;
  char root[] = "/private/tmp/colossus-helper-exec-XXXXXX";
  char broker[] = "/private/tmp/colossus-helper-broker-XXXXXX";
  if (!mkdtemp(root)) return 1;
  if (!mkdtemp(broker)) { rmdir(root); return 1; }
  const std::string bundle = std::string(root) + "/Browser.app";
  const std::string profile = std::string(root) + "/profile";
  const std::string marker = std::string(broker) + "/sealed-marker";
  char home_marker[] = "/private/tmp/colossus-helper-home-XXXXXX";
  int home_file = mkstemp(home_marker);
  if (home_file < 0 || write(home_file, "fixture", 7) != 7 || close(home_file)) return 1;
  std::error_code error;
  bool prepared = true;
  for (const auto* directory : {"home", "cache", "tmp"}) {
    fs::create_directories(fs::path(profile) / directory, error);
    prepared = prepared && !error;
  }
  int file = open(marker.c_str(), O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, 0600);
  prepared = prepared && file >= 0;
  if (file >= 0) {
    bool written = write(file, "fixture", 7) == 7;
    prepared = close(file) == 0 && written && prepared;
  }
  // Synthetic source input only. No retained audit-session identity is claimed.
  const colossus_mac_network_binding binding = {42, 54321, root, bundle.c_str(),
      "com.colossus.helper.fixture", profile.c_str(), broker, "/private/tmp"};
  bool passed = prepared;
  const struct { colossus_mac_network_role role; const char* suffix; } cases[] = {
      {COLOSSUS_MAC_ROLE_RENDERER, " (Renderer)"},
      {COLOSSUS_MAC_ROLE_GPU, " (GPU)"}, {COLOSSUS_MAC_ROLE_NETWORK, ""}};
  if (prepared) for (const auto& item : cases) {
    // Run every role even after one fails; each exact child remains reaped.
    bool result = Run(binding, image, marker, home_marker, item.role, item.suffix, version);
    passed = result && passed;
  }
  fs::remove_all(root, error);
  bool cleaned = !error && !fs::exists(root);
  fs::remove_all(broker, error);
  cleaned = !error && !fs::exists(broker) && cleaned;
  cleaned = unlink(home_marker) == 0 && cleaned;
  std::puts(cleaned ? "PASS toy helper children reaped and owned artifacts removed" :
                     "FAIL toy helper artifact cleanup");
  return passed && cleaned ? 0 : 2;
}
