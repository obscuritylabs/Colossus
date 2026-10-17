/* Owned no-browser test of the singleton socket allowance. No CEF, Keychain,
 * audit-session operations, delegated services, or remote network access. */
#include "network_envelope.h"
#include <arpa/inet.h>
#include <errno.h>
#include <limits.h>
#include <netinet/in.h>
#include <sandbox.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

extern int sandbox_init_with_parameters(const char *, unsigned long long,
                                        const char *const *, char **);
extern int sandbox_check(pid_t, const char *, int, ...);

static int joined(char *output, size_t capacity, const char *root,
                  const char *name) {
  int n = snprintf(output, capacity, "%s/%s", root, name);
  return n > 0 && (size_t)n < capacity;
}

static int unix_listener(const char *path, int allowed) {
  struct sockaddr_un address = {.sun_family = AF_UNIX};
  if (strlen(path) >= sizeof(address.sun_path)) return 0;
  strcpy(address.sun_path, path);
  address.sun_len = (unsigned char)sizeof(address);
  int fd = socket(AF_UNIX, SOCK_STREAM, 0);
  if (fd < 0) return 0;
  int result = bind(fd, (struct sockaddr *)&address, sizeof(address));
  int saved = errno;
  int passed = allowed ? result == 0 && listen(fd, 1) == 0 :
      result == -1 && (saved == EPERM || saved == EACCES);
  close(fd);
  if (result == 0 && unlink(path) != 0) passed = 0;
  return passed;
}

static int inet_bind_denied(int family) {
  int fd = socket(family, SOCK_STREAM, 0);
  if (fd < 0) return 0;
  int result;
  if (family == AF_INET) {
    struct sockaddr_in address = {.sin_len = sizeof(address),
      .sin_family = AF_INET, .sin_addr.s_addr = htonl(INADDR_LOOPBACK)};
    result = bind(fd, (struct sockaddr *)&address, sizeof(address));
  } else {
    struct sockaddr_in6 address = {.sin6_len = sizeof(address),
      .sin6_family = AF_INET6, .sin6_addr = IN6ADDR_LOOPBACK_INIT};
    result = bind(fd, (struct sockaddr *)&address, sizeof(address));
  }
  int saved = errno;
  close(fd);
  return result == -1 && (saved == EPERM || saved == EACCES);
}

static int wait_owned(pid_t child) {
  int status = 0;
  for (int attempt = 0; attempt < 600; ++attempt) {
    pid_t reaped = waitpid(child, &status, WNOHANG);
    if (reaped == child) {
      if (WIFEXITED(status) && WEXITSTATUS(status) == 0) return 1;
      fprintf(stderr, "owned Unix bind test category=%d\n",
              WIFEXITED(status) ? WEXITSTATUS(status) : -1);
      return 0;
    }
    if (reaped < 0 && errno != EINTR) return 0;
    struct timespec delay = {.tv_nsec = 10000000};
    nanosleep(&delay, NULL);
  }
  kill(child, SIGKILL);
  while (waitpid(child, &status, 0) < 0 && errno == EINTR) {}
  fputs("owned Unix bind test timed out\n", stderr);
  return 0;
}

int main(void) {
  char root[] = "/private/tmp/colossus-unix-XXXXXX";
  char broker[] = "/private/tmp/colossus-unix-broker-XXXXXX";
  if (!mkdtemp(root)) return 1;
  if (!mkdtemp(broker)) { rmdir(root); return 1; }
  char profile[PATH_MAX], temporary[PATH_MAX], bundle[PATH_MAX],
      allowed[PATH_MAX], denied[PATH_MAX], fronts[5][PATH_MAX];
  int profile_created = 0, temporary_created = 0, passed = 0;
  char *source = NULL;
  if (!joined(profile, sizeof(profile), root, "profile") ||
      !joined(bundle, sizeof(bundle), root, "Browser.app") ||
      !joined(temporary, sizeof(temporary), profile, "tmp") ||
      !joined(allowed, sizeof(allowed), temporary, "SingletonSocket") ||
      !joined(denied, sizeof(denied), profile, "OtherSocket")) goto done;
  if (mkdir(profile, 0700) != 0) goto done;
  profile_created = 1;
  if (mkdir(temporary, 0700) != 0) goto done;
  temporary_created = 1;
  static const char *const suffixes[] = {
      "", " (Renderer)", " (GPU)", " (Plugin)", " (Alerts)"};
  const char *entries[5];
  for (size_t i = 0; i < 5; ++i) {
    int n = snprintf(fronts[i], sizeof(fronts[i]),
      "%s/Contents/Frameworks/Colossus Browser Helper%s.app/Contents/MacOS/Colossus Browser Helper%s",
      bundle, suffixes[i], suffixes[i]);
    if (n <= 0 || (size_t)n >= sizeof(fronts[i])) goto done;
    entries[i] = fronts[i];
  }
  /* This binding is source-test data, not a retained session or lifetime claim. */
  struct colossus_mac_network_binding binding = {
      42, 54321, root, bundle, "com.colossus.browser.fixture",
      profile, broker, "/private/tmp"};
  if (!colossus_mac_main_network_policy(&binding, entries, 5, &source)) goto done;
  pid_t child = fork();
  if (child < 0) goto done;
  if (child == 0) {
    alarm(5);
    char *error = NULL;
    const char *parameters[] = {NULL};
    if (sandbox_init_with_parameters(source, 0, parameters, &error) != 0 ||
        sandbox_check(getpid(), NULL, 0) != 1) _exit(10);
    if (!unix_listener(allowed, 1)) _exit(11);
    /* This directory is writable: failure must come from the socket boundary. */
    if (!unix_listener(denied, 0)) _exit(12);
    if (!inet_bind_denied(AF_INET)) _exit(13);
    if (!inet_bind_denied(AF_INET6)) _exit(14);
    _exit(0);
  }
  passed = wait_owned(child);
done:
  free(source);
  if (temporary_created) {
    if (unlink(allowed) != 0 && errno != ENOENT) passed = 0;
    if (rmdir(temporary) != 0) passed = 0;
  }
  if (profile_created) {
    if (unlink(denied) != 0 && errno != ENOENT) passed = 0;
    if (rmdir(profile) != 0) passed = 0;
  }
  if (rmdir(broker) != 0 || rmdir(root) != 0) passed = 0;
  if (!passed) return 1;
  puts("main Seatbelt permits owned profile/tmp Unix bind/listen; other profile Unix sockets and IPv4/IPv6 listeners remain denied; child and socket artifacts removed");
  return 0;
}
