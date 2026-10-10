/* Disposable OS denial probe for the generated main profile. It never runs
 * Chromium, opens a Keychain, supplies an audit port, or changes global policy. */
#include "network_envelope.h"
#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <netinet/in.h>
#include <sandbox.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

extern int sandbox_init_with_parameters(const char *, unsigned long long,
                                        const char *const *, char **);
extern int sandbox_check(pid_t, const char *, int, ...);

static int listener(unsigned short *port) {
  int fd = socket(AF_INET, SOCK_STREAM, 0);
  if (fd < 0) return -1;
  struct sockaddr_in address = {.sin_family = AF_INET,
                                .sin_addr.s_addr = htonl(INADDR_LOOPBACK)};
  if (bind(fd, (struct sockaddr *)&address, sizeof(address)) || listen(fd, 2)) {
    fprintf(stderr, "fixture listener unavailable: errno=%d\n", errno);
    close(fd);
    return -1;
  }
  socklen_t length = sizeof(address);
  if (getsockname(fd, (struct sockaddr *)&address, &length)) {
    close(fd);
    return -1;
  }
  *port = ntohs(address.sin_port);
  return fd;
}

static int connect_to(unsigned short port) {
  int fd = socket(AF_INET, SOCK_STREAM, 0);
  if (fd < 0) return -1;
  struct sockaddr_in address = {.sin_family = AF_INET,
                                .sin_addr.s_addr = htonl(INADDR_LOOPBACK),
                                .sin_port = htons(port)};
  int result = connect(fd, (struct sockaddr *)&address, sizeof(address));
  int saved = errno;
  close(fd);
  errno = saved;
  return result;
}

static int append(char *output, size_t capacity, const char *parent,
                  const char *child) {
  int n = snprintf(output, capacity, "%s/%s", parent, child);
  return n > 0 && (size_t)n < capacity;
}

static int helper_denial(const char *source, unsigned short allowed,
                         unsigned short denied, const char *label,
                         int selected_permitted) {
  pid_t child = fork();
  if (child < 0) return 0;
  if (child == 0) {
    char *error = NULL;
    const char *parameters[] = {NULL};
    if (sandbox_init_with_parameters(source, 0, parameters, &error) != 0 ||
        sandbox_check(getpid(), NULL, 0) != 1) _exit(10);
    int selected = connect_to(allowed);
    if ((selected == 0) != selected_permitted)
      _exit(selected < 0 && errno > 0 && errno < 128 ? 100 + errno : 11);
    if (connect_to(denied) == 0) _exit(12);
    _exit(0);
  }
  int status = 0;
  int reaped = 0;
  for (int attempt = 0; attempt < 1000; ++attempt) {
    pid_t waited = waitpid(child, &status, WNOHANG);
    if (waited == child) { reaped = 1; break; }
    if (waited < 0) return 0;
    struct timespec delay = {.tv_sec = 0, .tv_nsec = 10000000};
    nanosleep(&delay, NULL);
  }
  if (!reaped) {
    kill(child, SIGKILL);
    waitpid(child, &status, 0);
    fprintf(stderr, "%s network denial child timed out\n", label);
    return 0;
  }
  if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) {
    fprintf(stderr, "%s network denial child category=%d\n", label,
            WIFEXITED(status) ? WEXITSTATUS(status) : -1);
    return 0;
  }
  return 1;
}

int main(void) {
  char root[] = "/private/tmp/colossus-network-denial-XXXXXX";
  char broker[] = "/private/tmp/colossus-broker-denial-XXXXXX";
  if (!mkdtemp(root) || !mkdtemp(broker)) return 1;
  char profile[PATH_MAX], bundle[PATH_MAX], marker[PATH_MAX],
       write_marker[PATH_MAX], fronts[5][PATH_MAX];
  if (!append(profile, sizeof(profile), root, "profile") ||
      !append(bundle, sizeof(bundle), root, "Browser.app") ||
      !append(marker, sizeof(marker), broker, "sealed-marker") ||
      !append(write_marker, sizeof(write_marker), broker, "denied-write") ||
      mkdir(profile, 0700) != 0) return 2;
  int file = open(marker, O_WRONLY | O_CREAT | O_EXCL, 0600);
  if (file < 0 || write(file, "fixture", 7) != 7 || close(file) != 0) return 2;
  static const char *const suffixes[] = {
      "", " (Alerts)", " (GPU)", " (Plugin)", " (Renderer)"};
  const char *entries[5];
  for (size_t i = 0; i < 5; ++i) {
    int n = snprintf(fronts[i], sizeof(fronts[i]),
        "%s/Contents/Frameworks/Colossus Browser Helper%s.app/Contents/MacOS/Colossus Browser Helper%s",
        bundle, suffixes[i], suffixes[i]);
    if (n <= 0 || (size_t)n >= sizeof(fronts[i])) return 2;
    entries[i] = fronts[i];
  }
  unsigned short allowed, denied;
  int allow_listener = listener(&allowed), deny_listener = listener(&denied);
  if (allow_listener < 0 || deny_listener < 0) return 2;
  struct colossus_mac_network_binding binding = {
      42, allowed, root, bundle, "com.obscuritylabs.colossus.native-browser-host.preview",
      profile, broker, "/Users/fixture"};
  char *source = NULL;
  if (!colossus_mac_main_network_policy(&binding, entries, 5, &source)) {
    fputs("fixture policy builder denied its fixed binding\n", stderr);
    return 2;
  }
  pid_t child = fork();
  if (child < 0) return 2;
  if (child == 0) {
    close(allow_listener);
    close(deny_listener);
    char *error = NULL;
    const char *parameters[] = {NULL};
    if (sandbox_init_with_parameters(source, 0, parameters, &error) != 0 ||
        sandbox_check(getpid(), NULL, 0) != 1) _exit(10);
    if (connect_to(allowed) != 0) _exit(11);
    if (connect_to(denied) == 0) _exit(12);
    int access = open(marker, O_RDONLY | O_NOFOLLOW);
    if (access >= 0) { close(access); _exit(13); }
    int write_test = open(write_marker, O_WRONLY | O_CREAT | O_EXCL, 0600);
    if (write_test >= 0) { close(write_test); _exit(14); }
    _exit(0);
  }
  free(source);
  int status = 0;
  int reaped = 0;
  for (int attempt = 0; attempt < 1000; ++attempt) {
    pid_t waited = waitpid(child, &status, WNOHANG);
    if (waited == child) { reaped = 1; break; }
    if (waited < 0) return 3;
    struct timespec delay = {.tv_sec = 0, .tv_nsec = 10000000};
    nanosleep(&delay, NULL);
  }
  if (!reaped) {
    kill(child, SIGKILL);
    waitpid(child, &status, 0);
    return 3;
  }
  if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) {
    fprintf(stderr, "network denial child category=%d\n",
            WIFEXITED(status) ? WEXITSTATUS(status) : -1);
    return 4;
  }

  const struct {
    enum colossus_mac_network_role role;
    const char *front;
    const char *label;
    int selected_permitted;
  } helper_cases[] = {
      {COLOSSUS_MAC_ROLE_NETWORK, fronts[0], "network helper", 1},
      {COLOSSUS_MAC_ROLE_RENDERER, fronts[4], "renderer helper", 0},
      {COLOSSUS_MAC_ROLE_PROXY_RESOLVER, fronts[0], "proxy resolver helper", 0},
      {COLOSSUS_MAC_ROLE_PRINT_BACKEND, fronts[0], "print backend helper", 0},
  };
  for (size_t i = 0; i < sizeof(helper_cases) / sizeof(helper_cases[0]); ++i) {
    char body[PATH_MAX];
    int length = snprintf(body, sizeof(body), "%s Body", helper_cases[i].front);
    if (length <= 0 || (size_t)length >= sizeof(body) ||
        !colossus_mac_helper_body_policy(helper_cases[i].role, &binding,
                                         helper_cases[i].front, body,
                                         getpid(), 2600, &source)) return 6;
    int passed = helper_denial(source, allowed, denied, helper_cases[i].label,
                               helper_cases[i].selected_permitted);
    free(source);
    if (!passed) return 7;
  }
  close(allow_listener);
  close(deny_listener);
  if (unlink(marker) || rmdir(profile) || rmdir(broker) || rmdir(root)) return 5;
  puts("main and network-helper Seatbelt permit only the selected fixture port; renderer, proxy-resolver and print helpers deny it; broker files remain protected");
  return 0;
}
