/* Actual main-envelope exec inheritance, with only this fixed toy executable.
 * No CEF, Keychain, audit port, launchd job or global policy operation is used. */
#include "network_envelope.h"
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <mach-o/dyld.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

extern int sandbox_init_with_parameters(const char *, unsigned long long,
                                        const char *const *, char **);
extern void sandbox_free_error(char *);
extern int sandbox_check(pid_t, const char *, int, ...);
#if defined(COLOSSUS_TEST_APPKIT_INITIALIZATION)
extern int colossus_appkit_fixture_initialize(void);
#endif
#if defined(COLOSSUS_TEST_DARWIN_DIRECTORY_QUERY)
extern int colossus_darwin_fixture_before(char [3][PATH_MAX], struct stat [3]);
extern int colossus_darwin_fixture_body(char *const [3], const char *);
extern int colossus_darwin_fixture_after(char [3][PATH_MAX], const struct stat [3]);
#endif

static int joined(char output[PATH_MAX], const char *parent, const char *leaf) {
  int size = snprintf(output, PATH_MAX, "%s/%s", parent, leaf);
  return size > 0 && size < PATH_MAX;
}

static int copy_own_image(const char *source, const char *destination) {
  int input = open(source, O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
  if (input < 0) return 0;
  struct stat image;
  if (fstat(input, &image) || !S_ISREG(image.st_mode) || image.st_size <= 0 ||
      image.st_size > 16 * 1024 * 1024) { close(input); return 0; }
  int output = open(destination, O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, 0700);
  int valid = output >= 0;
  char buffer[4096];
  off_t copied = 0;
  while (valid && copied < image.st_size) {
    ssize_t count = read(input, buffer, sizeof(buffer));
    if (count < 0 && errno == EINTR) continue;
    if (count <= 0) { valid = 0; break; }
    ssize_t written = 0;
    while (written < count) {
      ssize_t size = write(output, buffer + written, (size_t)(count - written));
      if (size < 0 && errno == EINTR) continue;
      if (size <= 0) { valid = 0; break; }
      written += size;
    }
    copied += count;
  }
  if (output >= 0 && close(output)) valid = 0;
  if (close(input)) valid = 0;
  return valid && copied == image.st_size;
}

static int body(const char *marker, const char *wrong, const char *sibling) {
  if (sandbox_check(getpid(), NULL, 0) != 1) return 31;
  char raw[PATH_MAX], resolved[PATH_MAX];
  uint32_t capacity = sizeof(raw);
  if (_NSGetExecutablePath(raw, &capacity) || !realpath(raw, resolved)) {
    fprintf(stderr, "own executable canonicalization failed: errno=%d\n", errno);
    return 35;
  }
  int denied = open(marker, O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
  if (denied >= 0) { close(denied); return 32; }
  if (errno != EACCES && errno != EPERM) return 33;
  denied = open(sibling, O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
  if (denied >= 0) { close(denied); return 36; }
  if (errno != EACCES && errno != EPERM) return 37;
  char *arguments[] = {(char *)wrong, "--unexpected-body", NULL};
  char *environment[] = {NULL};
  execve(wrong, arguments, environment);
  if (errno != EACCES && errno != EPERM) return 34;
#if defined(COLOSSUS_TEST_APPKIT_INITIALIZATION)
  int appkit = colossus_appkit_fixture_initialize();
  if (appkit) return appkit;
#endif
  puts("PASS fixed main body inherited Seatbelt; own realpath succeeded; broker/sibling data and wrong exec denied");
  return 0;
}

static int wait_owned(pid_t child) {
  int status = 0;
  for (unsigned pass = 0; pass < 600; ++pass) {
    pid_t result = waitpid(child, &status, WNOHANG);
    if (result == child) {
      if (WIFEXITED(status) && WEXITSTATUS(status) == 0) return 1;
      fprintf(stderr, "main-body fixture child category=%d\n",
              WIFEXITED(status) ? WEXITSTATUS(status) : -1);
      return 0;
    }
    if (result < 0 && errno != EINTR) return 0;
    struct timespec delay = {.tv_sec = 0, .tv_nsec = 10000000};
    nanosleep(&delay, NULL);
  }
  kill(child, SIGKILL);
  while (waitpid(child, &status, 0) < 0 && errno == EINTR) {}
  return 0;
}

int main(int argc, char **argv) {
#if defined(COLOSSUS_TEST_DARWIN_DIRECTORY_QUERY)
  if (argc == 9 && strcmp(argv[1], "--owned-body") == 0) {
    int result = body(argv[2], argv[3], argv[4]);
    return result ? result : colossus_darwin_fixture_body(argv + 5, argv[8]);
  }
#endif
  if (argc == 5 && strcmp(argv[1], "--owned-body") == 0)
    return body(argv[2], argv[3], argv[4]);
  if (argc != 1) return 30;
  umask(0077);
  char raw[PATH_MAX], executable[PATH_MAX];
  uint32_t capacity = sizeof(raw);
  if (_NSGetExecutablePath(raw, &capacity) || !realpath(raw, executable)) return 1;
  char root[] = "/private/tmp/colossus-main-exec-XXXXXX";
  char broker[] = "/private/tmp/colossus-main-exec-broker-XXXXXX";
  if (!mkdtemp(root) || !mkdtemp(broker)) return 2;
  char sibling[] = "/private/tmp/colossus-main-exec-sibling-XXXXXX";
  int sibling_file = mkstemp(sibling);
  if (sibling_file < 0 || write(sibling_file, "fixture", 7) != 7 || close(sibling_file)) return 2;
  char bundle[PATH_MAX], contents[PATH_MAX], macos[PATH_MAX],
       profile[PATH_MAX], marker[PATH_MAX], host[PATH_MAX], wrong[PATH_MAX];
  int valid = joined(bundle, root, "Browser.app") &&
    joined(contents, bundle, "Contents") && joined(macos, contents, "MacOS") &&
    joined(profile, root, "profile") && joined(marker, broker, "sealed-marker") &&
    joined(host, macos, "colossus-native-browser-host Body") &&
    joined(wrong, macos, "colossus-native-browser-host Wrong");
  if (!valid) return 3;
  if (mkdir(bundle, 0700) || mkdir(contents, 0700) ||
      mkdir(macos, 0700) || mkdir(profile, 0700)) return 4;
  int file = open(marker, O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, 0600);
  if (file < 0 || write(file, "fixture", 7) != 7 || close(file) ||
      !copy_own_image(executable, host) || !copy_own_image(executable, wrong)) return 5;
  char fronts[5][PATH_MAX];
  const char *entries[5];
  const char *suffixes[] = {"", " (Alerts)", " (GPU)", " (Plugin)", " (Renderer)"};
  for (size_t i = 0; i < 5; ++i) {
    int count = snprintf(fronts[i], sizeof(fronts[i]),
      "%s/Contents/Frameworks/Colossus Browser Helper%s.app/Contents/MacOS/Colossus Browser Helper%s",
      bundle, suffixes[i], suffixes[i]);
    if (count <= 0 || (size_t)count >= sizeof(fronts[i])) return 6;
    entries[i] = fronts[i];
  }
  /* This synthetic non-browser binding tests exec/network policy only. The ASID
   * field is builder input; no ownership or audit-session receipt is asserted. */
  struct colossus_mac_network_binding binding = {
    42, 54321, root, bundle, "com.obscuritylabs.colossus.native-body.fixture",
    profile, broker, "/private/tmp"};
  char *source = NULL;
  if (!colossus_mac_main_network_policy(&binding, entries, 5, &source)) return 7;
#if defined(COLOSSUS_TEST_DARWIN_DIRECTORY_QUERY)
  char directories[3][PATH_MAX];
  struct stat before[3];
  if (!colossus_darwin_fixture_before(directories, before)) return 9;
#endif
  pid_t child = fork();
  if (child == 0) {
    alarm(5);
    char *error = NULL;
    const char *parameters[] = {NULL};
    int applied = sandbox_init_with_parameters(source, 0, parameters, &error);
    if (error) sandbox_free_error(error);
    if (applied || sandbox_check(getpid(), NULL, 0) != 1) _exit(20);
    char *arguments[] = {host, "--owned-body", marker, wrong, sibling,
#if defined(COLOSSUS_TEST_DARWIN_DIRECTORY_QUERY)
      directories[0], directories[1], directories[2], profile,
#endif
      NULL};
    char *environment[] = {NULL};
    execve(host, arguments, environment);
    _exit(21);
  }
  free(source);
  int passed = child > 0 && wait_owned(child);
#if defined(COLOSSUS_TEST_DARWIN_DIRECTORY_QUERY)
  passed = colossus_darwin_fixture_after(directories, before) && passed;
#endif
  int cleaned = !unlink(host) && !unlink(wrong) && !unlink(marker) && !unlink(sibling) &&
    !rmdir(profile) && !rmdir(macos) && !rmdir(contents) && !rmdir(bundle) &&
    !rmdir(broker) && !rmdir(root);
  if (passed && cleaned) puts("PASS main-body exec fixture; browser=false keychain=false audit_ports=false");
  return passed && cleaned ? 0 : 8;
}
