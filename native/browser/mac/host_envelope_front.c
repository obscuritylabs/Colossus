/* The dedicated browser's first userspace entry. Only libSystem, libsandbox,
 * and the pure, pinned policy builder may be linked here. No CEF, Objective-C,
 * Security framework, Rust or dynamic policy input is loaded before Seatbelt. */
#include "network_envelope.h"

#include <bsm/audit.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <mach-o/dyld.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

extern char **environ;
extern int sandbox_init_with_parameters(const char *, uint64_t,
                                        const char *const *, char **);
extern void sandbox_free_error(char *);

#define RESOURCE "Contents/Resources/colossus-network-envelope.policy"
#define HOST "Contents/MacOS/colossus-native-browser-host"
#define POLICY_LIMIT (32U * 1024U)

static int child_arguments(int argc, char **argv) {
  if (argc != 5) return 0;
  for (int i = 1; i != 5; ++i) {
    if (argv[i][0] != '0' + i + 2 || argv[i][1] != '\0') return 0;
  }
  return 1;
}

static int joined(char output[PATH_MAX], const char *parent, const char *leaf) {
  int n = snprintf(output, PATH_MAX, "%s/%s", parent, leaf);
  return n > 0 && n < PATH_MAX;
}

static int canonical(const char *path) {
  char resolved[PATH_MAX];
  return realpath(path, resolved) != NULL && strcmp(path, resolved) == 0;
}

static int private_directory(const char *path) {
  struct stat metadata;
  return canonical(path) && lstat(path, &metadata) == 0 &&
         S_ISDIR(metadata.st_mode) && metadata.st_uid == geteuid() &&
         (metadata.st_mode & 07777) == 0700 && metadata.st_nlink >= 2;
}

static int fixed_environment(const struct colossus_mac_network_binding *binding) {
  char home[PATH_MAX], config[PATH_MAX], data[PATH_MAX], temp[PATH_MAX];
  if (!joined(home, binding->profile_root, "home") ||
      !joined(config, home, ".config") ||
      !joined(data, home, ".local/share") ||
      !joined(temp, binding->profile_root, "tmp") ||
      !private_directory(home) || !private_directory(config) ||
      !private_directory(data) || !private_directory(temp)) return 0;
  struct expected { const char *name; const char *value; } allow[] = {
      {"HOME", home}, {"XDG_CONFIG_HOME", config},
      {"XDG_DATA_HOME", data}, {"TMPDIR", temp},
      {"PATH", "/usr/bin:/bin"}, {"LANG", "en_US.UTF-8"},
      {"MallocNanoZone", "0"},
  };
  size_t seen = 0;
  for (char **entry = environ; *entry; ++entry) {
    const char *equals = strchr(*entry, '=');
    if (!equals) return 0;
    size_t length = (size_t)(equals - *entry);
    size_t i = 0;
    for (; i < sizeof(allow) / sizeof(allow[0]); ++i) {
      if (strlen(allow[i].name) == length &&
          memcmp(*entry, allow[i].name, length) == 0 &&
          strcmp(equals + 1, allow[i].value) == 0) break;
    }
    if (i == sizeof(allow) / sizeof(allow[0]) || (seen & (1U << i))) return 0;
    seen |= 1U << i;
  }
  return seen == (1U << (sizeof(allow) / sizeof(allow[0]))) - 1U;
}

static int read_binding(const char *bundle, char storage[POLICY_LIMIT + 1],
                        struct colossus_mac_network_binding *binding) {
  char path[PATH_MAX];
  if (!joined(path, bundle, RESOURCE)) return 0;
  int fd = open(path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
  if (fd < 0) return 0;
  struct stat before, after;
  int ok = fstat(fd, &before) == 0 && S_ISREG(before.st_mode) &&
           before.st_uid == geteuid() && before.st_nlink == 1 &&
           (before.st_mode & 07777) == 0400 && before.st_size > 0 &&
           before.st_size <= POLICY_LIMIT;
  size_t remaining = ok ? (size_t)before.st_size : 0;
  char *cursor = storage;
  while (ok && remaining) {
    ssize_t n = read(fd, cursor, remaining);
    if (n < 0 && errno == EINTR) continue;
    if (n <= 0) { ok = 0; break; }
    cursor += n;
    remaining -= (size_t)n;
  }
  if (ok) {
    ok = fstat(fd, &after) == 0 && before.st_dev == after.st_dev &&
         before.st_ino == after.st_ino && before.st_size == after.st_size &&
         before.st_mtimespec.tv_sec == after.st_mtimespec.tv_sec &&
         before.st_mtimespec.tv_nsec == after.st_mtimespec.tv_nsec;
  }
  if (close(fd) != 0) ok = 0;
  if (!ok) return 0;
  storage[before.st_size] = '\0';
  return colossus_mac_network_binding_parse(storage, (size_t)before.st_size,
                                             binding);
}

int main(int argc, char **argv) {
  if (getuid() == 0 || getuid() != geteuid() || !child_arguments(argc, argv))
    return 70;
  char raw[PATH_MAX], executable[PATH_MAX];
  uint32_t size = sizeof(raw);
  if (_NSGetExecutablePath(raw, &size) != 0 || !realpath(raw, executable))
    return 71;
  size_t executable_length = strlen(executable);
  if (executable_length <= strlen(HOST)) return 71;
  size_t prefix = executable_length - strlen(HOST);
  if (strcmp(executable + prefix, HOST) != 0 || prefix >= PATH_MAX)
    return 71;
  char bundle[PATH_MAX];
  memcpy(bundle, executable, prefix);
  bundle[prefix - 1] = '\0';
  char storage[POLICY_LIMIT + 1];
  struct colossus_mac_network_binding binding;
  if (!read_binding(bundle, storage, &binding) ||
      strcmp(bundle, binding.bundle_path) != 0 ||
      !private_directory(binding.allocation_root) ||
      !private_directory(binding.profile_root) ||
      !canonical(binding.broker_root) ||
      !canonical(binding.personal_home) || !fixed_environment(&binding))
    return 72;
  auditinfo_addr_t audit;
  if (getaudit_addr(&audit, sizeof(audit)) != 0 ||
      (uint32_t)audit.ai_asid != binding.audit_session || getppid() <= 1)
    return 73;
  char body[PATH_MAX];
  int body_length = snprintf(body, sizeof(body), "%s Body", executable);
  if (body_length <= 0 || body_length >= (int)sizeof(body) ||
      !canonical(body)) return 74;
  struct stat image;
  if (lstat(body, &image) != 0 || !S_ISREG(image.st_mode) ||
      image.st_uid != geteuid() || image.st_nlink != 1 ||
      (image.st_mode & 0022)) return 74;
  static const char *const suffixes[] = {
      "", " (Alerts)", " (GPU)", " (Plugin)", " (Renderer)"};
  const char *fronts[5];
  char names[5][PATH_MAX];
  for (size_t i = 0; i != 5; ++i) {
    int n = snprintf(names[i], sizeof(names[i]),
        "%s/Contents/Frameworks/Colossus Browser Helper%s.app/Contents/MacOS/Colossus Browser Helper%s",
        bundle, suffixes[i], suffixes[i]);
    if (n <= 0 || n >= (int)sizeof(names[i]) || !canonical(names[i])) return 75;
    fronts[i] = names[i];
  }
  char *source = NULL, *error = NULL;
  if (!colossus_mac_main_network_policy(&binding, fronts, 5, &source)) return 76;
  const char *parameters[] = {NULL};
  int result = sandbox_init_with_parameters(source, 0, parameters, &error);
  free(source);
  if (error) sandbox_free_error(error);
  if (result != 0) return 77;
  memset(storage, 0, sizeof(storage));
  argv[0] = body;
  execve(body, argv, environ);
  return 78;
}
