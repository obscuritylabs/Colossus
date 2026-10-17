/* Public confstr queries only, in an exec'ed no-browser main-envelope child.
 * No personal-cache files are created/read, private MIG calls or Keychain APIs
 * are used. The parent checks existing current-UID directory metadata around it. */
#include <arpa/inet.h>
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <netinet/in.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/xattr.h>
#include <unistd.h>

extern int sandbox_check(pid_t, const char *, int, ...);
static const int queries[3] = {
  _CS_DARWIN_USER_CACHE_DIR, _CS_DARWIN_USER_DIR, _CS_DARWIN_USER_TEMP_DIR};

static int current(int query, char path[PATH_MAX]) {
  char raw[PATH_MAX];
  size_t length = confstr(query, raw, sizeof(raw));
  if (!length || length > sizeof(raw) || !realpath(raw, path)) return 0;
  return strncmp(path, "/private/var/folders/", 21) == 0;
}

int colossus_darwin_fixture_before(char paths[3][PATH_MAX], struct stat before[3]) {
  /* Resolve every baseline query before taking any "before child" snapshot. */
  for (size_t i = 0; i < 3; ++i)
    if (!current(queries[i], paths[i])) return 0;
  for (size_t i = 0; i < 3; ++i) {
    if (lstat(paths[i], before + i) ||
        !S_ISDIR(before[i].st_mode) || before[i].st_uid != geteuid()) return 0;
  }
  return 1;
}

static int denied_errno(void) { return errno == EPERM || errno == EACCES; }

static int contents_denied(const char *path) {
  /* Path filter1, proven below with an allowed private directory contrast. */
  if (sandbox_check(getpid(), "file-read-data", 1, path) != 1 ||
      sandbox_check(getpid(), "file-read-xattr", 1, path) != 1 ||
      sandbox_check(getpid(), "file-write-create", 1, path) != 1) return 0;
  errno = 0;
  DIR *directory = opendir(path);
  if (directory) {
    errno = 0;
    struct dirent *entry = readdir(directory);
    int blocked = !entry && denied_errno();
    closedir(directory);
    if (!blocked) return 0;
  } else if (!denied_errno()) return 0;
  errno = 0;
  if (getxattr(path, "com.colossus.fixture-missing", NULL, 0, 0, 0) != -1 ||
      !denied_errno()) return 0;
  return 1;
}

int colossus_darwin_fixture_body(char *const paths[3], const char *profile) {
  if (sandbox_check(getpid(), "mach-lookup", 2, "com.apple.bsd.dirhelper") != 0)
    return 50;
  for (size_t i = 0; i < 3; ++i) {
    char actual[PATH_MAX];
    if (!current(queries[i], actual) || strcmp(actual, paths[i])) return 51;
    if (!contents_denied(paths[i])) return 52;
    /* Check the canonical /var alias too without reading any contents. */
    if (strncmp(paths[i], "/private/", 9) || !contents_denied(paths[i] + 8))
      return 53;
  }
  if (sandbox_check(getpid(), "file-read-data", 1, profile) != 0 ||
      sandbox_check(getpid(), "file-write-create", 1, profile) != 0) return 54;
  char marker[PATH_MAX];
  int length = snprintf(marker, sizeof(marker), "%s/fixture-write", profile);
  if (length <= 0 || (size_t)length >= sizeof(marker)) return 55;
  int fd = open(marker, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0600);
  if (fd < 0 || write(fd, "fixture", 7) != 7 || close(fd) || unlink(marker))
    return 56;
  /* Exact owned allocation path outside profile: a safe actual write denial. */
  strcpy(marker, profile);
  char *leaf = strrchr(marker, '/');
  if (!leaf) return 57;
  strcpy(leaf, "/outside-write");
  fd = open(marker, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0600);
  if (fd >= 0) { close(fd); unlink(marker); return 58; }
  if (!denied_errno()) return 59;
  const char *denied[] = {"com.apple.SecurityServer", "com.apple.securityd",
    "com.apple.pasteboard.1", "com.apple.cfprefsd.agent", "com.apple.tccd"};
  for (size_t i = 0; i < sizeof(denied) / sizeof(denied[0]); ++i)
    if (sandbox_check(getpid(), "mach-lookup", 2, denied[i]) != 1) return 60;
  int socket_fd = socket(AF_INET, SOCK_STREAM, 0);
  if (socket_fd < 0) return 61;
  struct sockaddr_in address = {.sin_len = sizeof(address), .sin_family = AF_INET,
    .sin_addr.s_addr = htonl(INADDR_LOOPBACK), .sin_port = htons(1)};
  int result = connect(socket_fd, (struct sockaddr *)&address, sizeof(address));
  int blocked = result == -1 && denied_errno();
  close(socket_fd);
  if (!blocked) return 62;
  puts("PASS public Darwin directory queries; shared directory enumeration/xattrs/writes denied for canonical and alias paths; profile-only write and forbidden services/network preserved");
  return 0;
}

static int times_equal(struct timespec a, struct timespec b) {
  return a.tv_sec == b.tv_sec && a.tv_nsec == b.tv_nsec;
}

int colossus_darwin_fixture_after(char paths[3][PATH_MAX], const struct stat before[3]) {
  for (size_t i = 0; i < 3; ++i) {
    struct stat after;
    if (lstat(paths[i], &after)) return 0;
    unsigned changed = (before[i].st_dev != after.st_dev ? 1U : 0U) |
      (before[i].st_ino != after.st_ino ? 2U : 0U) |
      (before[i].st_mode != after.st_mode ? 4U : 0U) |
      (before[i].st_uid != after.st_uid ? 8U : 0U) |
      (before[i].st_gid != after.st_gid ? 16U : 0U) |
      (before[i].st_nlink != after.st_nlink ? 32U : 0U) |
      (before[i].st_size != after.st_size ? 64U : 0U) |
      (!times_equal(before[i].st_birthtimespec, after.st_birthtimespec) ? 128U : 0U) |
      (!times_equal(before[i].st_atimespec, after.st_atimespec) ? 256U : 0U) |
      (!times_equal(before[i].st_mtimespec, after.st_mtimespec) ? 512U : 0U) |
      (!times_equal(before[i].st_ctimespec, after.st_ctimespec) ? 1024U : 0U) |
      (before[i].st_flags != after.st_flags ? 2048U : 0U);
    if (changed & 1024U)
      fprintf(stderr, "DIAGNOSTIC Darwin directory ctime drift: query=%zu attribution=unknown\n", i);
    if (changed & ~1024U) {
      fprintf(stderr, "Darwin directory metadata changed: query=%zu fields=%u\n", i, changed);
      return 0;
    }
  }
  puts("PASS existing current-UID Darwin directory identity/ownership/permissions/flags/size/birth/atime/mtime unchanged; ctime drift recorded separately");
  return 1;
}
