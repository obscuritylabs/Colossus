/* This negative containment witness deliberately allows posix_spawn to show its escape. */
#include <errno.h>
#include <sandbox.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

extern char **environ;

static void run_case(const char *self, const char *kind, int deny, short flags) {
  fflush(NULL);
  pid_t child = fork();
  if (child < 0) exit(10);
  if (!child) {
    alarm(10);
    if (deny) {
      char *error = NULL;
      const char *profile = deny == 1 ?
        "(version 1) (allow default) (deny syscall-unix (syscall-number SYS_setsid) (syscall-number SYS_setpgid))" :
        "(version 1) (allow default) (deny syscall-unix (syscall-number SYS_posix_spawn))";
      int result = sandbox_init(profile, 0, &error);
      if (error) sandbox_free_error(error);
      if (result != 0) _exit(11);
    }
    if (!strcmp(kind, "setsid") || !strcmp(kind, "setpgid")) {
      errno = 0;
      int result = !strcmp(kind, "setsid") ? setsid() : setpgid(0, 0);
      printf("DIRECT kind=%s deny=%d result=%d error=%d\n", kind, deny, result, errno);
    } else {
      posix_spawnattr_t attributes;
      if (posix_spawnattr_init(&attributes) != 0 ||
          posix_spawnattr_setflags(&attributes, flags) != 0 ||
          posix_spawnattr_setpgroup(&attributes, 0) != 0) _exit(12);
      pid_t spawned = -1;
      char *const args[] = {(char *)self, "spawn-identify", NULL};
      int result = posix_spawn(&spawned, self, NULL, &attributes, args, environ);
      printf("SPAWN kind=%s deny=%d result=%d flags=%u spawned=%d pgid=%d sid=%d\n", kind, deny,
             result, (unsigned short)flags, spawned, getpgrp(), getsid(0));
      posix_spawnattr_destroy(&attributes);
      if (!result) {
        int status = 0;
        if (waitpid(spawned, &status, 0) != spawned || !WIFEXITED(status) || WEXITSTATUS(status)) _exit(13);
        puts("SPAWN_REAPED");
      }
    }
    fflush(NULL);
    _exit(0);
  }
  int status = 0;
  if (waitpid(child, &status, 0) != child || !WIFEXITED(status) || WEXITSTATUS(status)) exit(14);
  printf("CASE_REAPED kind=%s deny=%d\n", kind, deny);
}

void syscall_boundary_probe(const char *self) {
  run_case(self, "setsid", 0, 0);
  run_case(self, "setpgid", 0, 0);
  run_case(self, "setsid", 1, 0);
  run_case(self, "setpgid", 1, 0);
  run_case(self, "setsid-spawn", 1, POSIX_SPAWN_SETSID);
  run_case(self, "setpgroup-spawn", 1, POSIX_SPAWN_SETPGROUP);
  run_case(self, "default-spawn", 1, 0);
  run_case(self, "default-spawn", 2, 0);
  puts("SYSCALL_SUITE_COMPLETE cases=8 whole_tree_containment=0");
}
