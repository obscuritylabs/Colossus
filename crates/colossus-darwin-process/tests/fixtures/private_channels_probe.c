/* Fixed owned bootstrap fixture. It creates no descendants except its own exec. */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>

static int descriptor_count(void) {
  int count = 0;
  for (int descriptor = 0; descriptor < 4096; ++descriptor)
    if (fcntl(descriptor, F_GETFD) >= 0) ++count;
  return count;
}

int main(int argc, char **argv) {
  alarm(5);
  setvbuf(stdout, NULL, _IOLBF, 0);
  setvbuf(stderr, NULL, _IOLBF, 0);
  if (argc != 2) return 2;
  if (!strcmp(argv[1], "execed")) {
    for (int descriptor = 3; descriptor <= 6; ++descriptor) {
      errno = 0;
      if (fcntl(descriptor, F_GETFD) != -1 || errno != EBADF) return 3;
    }
    int count = descriptor_count();
    printf("HELPER_EXEC channels_closed=4 fd_count=%d\n", count);
    return count == 3 ? 0 : 4;
  }
  struct stat input;
  char byte;
  if (fstat(STDIN_FILENO, &input) || !S_ISCHR(input.st_mode) || read(STDIN_FILENO, &byte, 1) != 0) return 5;
  int count = descriptor_count();
  if (count != 7) return 6;
  puts("STDOUT_ONLY");
  fputs("STDERR_ONLY\n", stderr);
  for (int descriptor = 3; descriptor <= 6; ++descriptor) {
    struct sockaddr_un address;
    socklen_t size = sizeof address;
    int kind = 0; socklen_t kind_size = sizeof kind;
    if (getsockname(descriptor, (struct sockaddr *)&address, &size) || address.sun_family != AF_UNIX ||
        getsockopt(descriptor, SOL_SOCKET, SO_TYPE, &kind, &kind_size) || kind != SOCK_STREAM) return 7;
    unsigned char identity = (unsigned char)descriptor;
    if (write(descriptor, &identity, 1) != 1) return 8;
  }
  if (!strcmp(argv[1], "hold")) {
    if (read(3, &byte, 1) != 1) return 9;
    return 10;
  }
  if (strcmp(argv[1], "channels")) return 11;
  for (int descriptor = 3; descriptor <= 6; ++descriptor) {
    if (read(descriptor, &byte, 1) != 1 || byte != descriptor + 64) return 12;
    int flags = fcntl(descriptor, F_GETFD);
    if (flags < 0 || fcntl(descriptor, F_SETFD, flags | FD_CLOEXEC)) return 13;
  }
  printf("BOOTSTRAP fd_order=3,4,5,6 socket_count=4 fd_count=%d\n", count);
  char *arguments[] = {argv[0], "execed", NULL};
  char *environment[] = {NULL};
  execve(argv[0], arguments, environment);
  return 14;
}
