/* Closed native fixture. No browser, network, GUI, arbitrary program or UID change. */
#include <errno.h>
#include <libproc.h>
#include <mach/mach.h>
#include <mach/task_info.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

void syscall_boundary_probe(const char *self);

static void identity(const char *kind) {
  audit_token_t token = {0};
  mach_msg_type_number_t count = TASK_AUDIT_TOKEN_COUNT;
  if (task_info(mach_task_self(), TASK_AUDIT_TOKEN, (task_info_t)&token, &count) != KERN_SUCCESS ||
      count != TASK_AUDIT_TOKEN_COUNT) exit(3);
  printf("%s pid=%d pgid=%d sid=%d asid=%u uid=%u\n", kind,
         getpid(), getpgrp(), getsid(0), token.val[6], getuid());
}

static void descriptors(void) {
  struct proc_fdinfo descriptors[128];
  int bytes = proc_pidinfo(getpid(), PROC_PIDLISTFDS, 0, descriptors, sizeof descriptors);
  if (bytes <= 0 || bytes % sizeof(struct proc_fdinfo)) exit(10);
  int count = bytes / sizeof(struct proc_fdinfo);
  int unexpected = 0;
  for (int index = 0; index < count; ++index) if (descriptors[index].proc_fd > 2) unexpected++;
  printf("DESCRIPTORS count=%d unexpected=%d\n", count, unexpected);
}

static int controlled(const char *self, int detached, int after_exec) {
  alarm(10);
  identity(after_exec ? "EXECED" : "READY");
  for (;;) {
    int command = getchar();
    if (command == EOF || command == 'X') return 0;
    if (command == 'P') puts("PONG");
    else if (command == 'E' && !detached) {
      execl(self, self, "after-exec", (char *)NULL);
      perror("fixed self exec");
      return 4;
    } else if (command == 'D' && detached) {
      errno = 0;
      pid_t result = setsid();
      printf("SETSID result=%d error=%d\n", result, errno);
      if (result < 0) return 5;
      identity("DETACHED");
    } else return 6;
  }
}

int main(int argc, char **argv) {
  setvbuf(stdout, NULL, _IOLBF, 0);
  if (argc != 2) return 2;
  char self[4096];
  if (!realpath(argv[0], self)) return 2;
  if (!strcmp(argv[1], "control") || !strcmp(argv[1], "after-exec"))
    return controlled(self, 0, !strcmp(argv[1], "after-exec"));
  if (!strcmp(argv[1], "spawn-identify")) { identity("SPAWNED"); return 0; }
  if (!strcmp(argv[1], "descriptors")) { descriptors(); return 0; }
  if (!strcmp(argv[1], "syscalls")) { syscall_boundary_probe(self); return 0; }
  if (!strcmp(argv[1], "detached")) {
    alarm(15);
    pid_t child = fork();
    if (child < 0) return 7;
    if (!child) _exit(controlled(self, 1, 0));
    int status = 0;
    if (waitpid(child, &status, 0) != child) return 8;
    printf("DESCENDANT_REAPED pid=%d signal=%d\n", child,
           WIFSIGNALED(status) ? WTERMSIG(status) : 0);
    return WIFEXITED(status) && WEXITSTATUS(status) == 0 ? 0 : 9;
  }
  return 2;
}
