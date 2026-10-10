/* Own fresh-ASID records only. No audit policy changes or all-sessions reads. */
#include <bsm/audit.h>
#include <bsm/audit_session.h>
#include <bsm/libbsm.h>
#include <errno.h>
#include <fcntl.h>
#include <libproc.h>
#include <mach/mach.h>
#include <mach/task_info.h>
#include <servers/bootstrap.h>
#include <poll.h>
#include <security/audit/audit_ioctl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/socket.h>
#include <sys/select.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <time.h>
#include <unistd.h>

struct receipt { unsigned int magic, pid, asid, owner_pid; unsigned long long rdev; };
static const unsigned int MAGIC = 0xC01055A5;

struct session_message {
  mach_msg_header_t header;
  mach_msg_body_t body;
  mach_msg_port_descriptor_t port;
  unsigned int magic, pid, asid;
};

static int publish_session(const char *service, unsigned int asid) {
  mach_port_t remote = MACH_PORT_NULL;
  if (bootstrap_look_up(bootstrap_port, service, &remote)) return -1;
  mach_port_t current = audit_session_self();
  if (!current) return -1;
  struct session_message message = {0};
  message.header.msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, 0) | MACH_MSGH_BITS_COMPLEX;
  message.header.msgh_remote_port = remote;
  message.header.msgh_size = sizeof message;
  message.header.msgh_id = 6501;
  message.body.msgh_descriptor_count = 1;
  message.port.name = current;
  message.port.disposition = MACH_MSG_TYPE_COPY_SEND;
  message.port.type = MACH_MSG_PORT_DESCRIPTOR;
  message.magic = MAGIC; message.pid = (unsigned int)getpid(); message.asid = asid;
  kern_return_t result = mach_msg(&message.header, MACH_SEND_MSG, sizeof message, 0, MACH_PORT_NULL, 0, MACH_PORT_NULL);
  mach_port_deallocate(mach_task_self(), current);
  mach_port_deallocate(mach_task_self(), remote);
  return result ? -1 : 0;
}

static int read_exact(int descriptor, unsigned char *bytes, size_t length) {
  size_t used = 0;
  for (int pass = 0; pass < 100 && used < length; ++pass) {
    ssize_t count = read(descriptor, bytes + used, length - used);
    if (count > 0) { used += (size_t)count; continue; }
    if (count < 0 && errno == EINTR) continue;
    if (count < 0 && errno == EAGAIN) {
      fd_set ready; FD_ZERO(&ready); FD_SET(descriptor, &ready);
      struct timeval timeout = {0, 100000};
      if (select(descriptor + 1, &ready, NULL, NULL, &timeout) >= 0) continue;
    }
    return -1;
  }
  return used == length ? 0 : -1;
}

static int record(int descriptor, unsigned int expected_asid) {
  unsigned char bytes[MAXAUDITDATA];
  if (read_exact(descriptor, bytes, 5)) return -1;
  unsigned int length = ((unsigned int)bytes[1] << 24) | ((unsigned int)bytes[2] << 16) |
    ((unsigned int)bytes[3] << 8) | bytes[4];
  if (length < 18 || length > sizeof bytes || read_exact(descriptor, bytes + 5, length - 5)) return -1;
  /* Native bytes are captured above before decoding; only an owned END is emitted. */
  int event = -1, owned = 0;
  size_t offset = 0;
  while (offset < length) {
    tokenstr_t token;
    if (au_fetch_tok(&token, bytes + offset, (int)(length - offset)) || !token.len || token.len > length - offset) return -1;
    switch (token.id) {
      case AUT_HEADER32: event = token.tt.hdr32.e_type; break;
      case AUT_HEADER32_EX: event = token.tt.hdr32_ex.e_type; break;
      case AUT_HEADER64: event = token.tt.hdr64.e_type; break;
      case AUT_HEADER64_EX: event = token.tt.hdr64_ex.e_type; break;
      case AUT_SUBJECT32: owned = token.tt.subj32.sid == expected_asid; break;
      case AUT_SUBJECT32_EX: owned = token.tt.subj32_ex.sid == expected_asid; break;
      case AUT_SUBJECT64: owned = token.tt.subj64.sid == expected_asid; break;
      case AUT_SUBJECT64_EX: owned = token.tt.subj64_ex.sid == expected_asid; break;
      default: break;
    }
    offset += token.len;
  }
  if (!owned) return -1;
  if (event == AUE_SESSION_END) {
    printf("owned_end_raw_bytes=%u\n", length);
  }
  return event;
}

static unsigned int own_asid(void) {
  audit_token_t token = {0};
  mach_msg_type_number_t count = TASK_AUDIT_TOKEN_COUNT;
  if (task_info(mach_task_self(), TASK_AUDIT_TOKEN, (task_info_t)&token, &count) || count != TASK_AUDIT_TOKEN_COUNT) exit(3);
  return token.val[6];
}

static int address(struct sockaddr_un *address, const char *path) {
  memset(address, 0, sizeof *address);
  address->sun_family = AF_UNIX;
  if (strlen(path) >= sizeof address->sun_path) return -1;
  strcpy(address->sun_path, path);
  address->sun_len = sizeof *address;
  return 0;
}

static int producer(const char *path, const char *service, int inherited, unsigned int owner_pid) {
  alarm(10);
  au_sdev_handle_t *device = inherited < 0 ? au_sdev_open(AU_SDEVF_NONBLOCK) : NULL;
  if (inherited < 0 && !device) return 4;
  int descriptor = inherited < 0 ? au_sdev_fd(device) : inherited;
  unsigned int all = 1;
  unsigned long long drops = 0;
  struct stat metadata;
  errno = 0;
  int flags_read = ioctl(descriptor, AUDITSDEV_GET_ALLSESSIONS, &all);
  int flags_error = errno;
  if (fstat(descriptor, &metadata) || !S_ISCHR(metadata.st_mode) ||
      ((fcntl(descriptor, F_GETFL) & O_ACCMODE) != O_RDONLY) ||
      (flags_read ? flags_error != EPERM : all != 0) ||
      ioctl(descriptor, AUDITSDEV_GET_DROPS, &drops) || drops) return 5;
  printf("all_sessions_query=%d errno=%d constructor_all_sessions=false\n", flags_read, flags_error);
  struct sockaddr_un target;
  if (address(&target, path)) return 6;
  int socket_fd = socket(AF_UNIX, SOCK_STREAM, 0);
  if (socket_fd < 0 || connect(socket_fd, (struct sockaddr *)&target, sizeof target)) return 7;
  struct receipt receipt = {MAGIC, (unsigned int)getpid(), own_asid(), owner_pid, (unsigned long long)metadata.st_rdev};
  if (service && publish_session(service, receipt.asid)) return 31;
  struct iovec vector = {&receipt, sizeof receipt};
  union { struct cmsghdr alignment; unsigned char bytes[CMSG_SPACE(sizeof(int))]; } control;
  memset(&control, 0, sizeof control);
  struct msghdr message = {0};
  message.msg_iov = &vector; message.msg_iovlen = 1;
  message.msg_control = control.bytes; message.msg_controllen = sizeof control.bytes;
  struct cmsghdr *header = CMSG_FIRSTHDR(&message);
  header->cmsg_level = SOL_SOCKET; header->cmsg_type = SCM_RIGHTS;
  header->cmsg_len = CMSG_LEN(sizeof(int));
  memcpy(CMSG_DATA(header), &descriptor, sizeof descriptor);
  if (sendmsg(socket_fd, &message, 0) != sizeof receipt) return 8;
  char acknowledgment;
  if (read(socket_fd, &acknowledgment, 1) != 1 || acknowledgment != 'A') return 9;
  pid_t child = fork();
  if (child < 0) return 10;
  if (!child) {
    close(descriptor); close(socket_fd);
    alarm(5);
    if (setsid() < 0) _exit(11);
    sleep(2);
    _exit(0);
  }
  if (write(socket_fd, &child, sizeof child) != sizeof child) return 12;
  if (read(socket_fd, &acknowledgment, 1) != 1 || acknowledgment != 'E') return 32;
  printf("producer_asid=%u child_pid=%d scoped_device=true transferred=true\n", receipt.asid, child);
  if (device) au_sdev_close(device); else close(descriptor);
  close(socket_fd);
  /* Intentionally exit before the detached child: END must wait for that child. */
  return 0;
}

static int observer(const char *path, int crash, const char *rust_path) {
  alarm(15);
  struct sockaddr_un own;
  if (address(&own, path)) return 13;
  int listener = socket(AF_UNIX, SOCK_STREAM, 0);
  if (listener < 0 || bind(listener, (struct sockaddr *)&own, sizeof own) || listen(listener, 1)) return 14;
  puts("OBSERVER_READY");
  int connection = accept(listener, NULL, NULL);
  if (connection < 0) return 15;
  uid_t uid; gid_t gid;
  if (getpeereid(connection, &uid, &gid) || uid != getuid()) return 16;
  pid_t peer = -1; socklen_t peer_size = sizeof peer;
  if (getsockopt(connection, SOL_LOCAL, LOCAL_PEERPID, &peer, &peer_size)) return 17;
  struct receipt receipt = {0};
  struct iovec vector = {&receipt, sizeof receipt};
  union { struct cmsghdr alignment; unsigned char bytes[CMSG_SPACE(sizeof(int))]; } control;
  memset(&control, 0, sizeof control);
  struct msghdr message = {0};
  message.msg_iov = &vector; message.msg_iovlen = 1;
  message.msg_control = control.bytes; message.msg_controllen = sizeof control.bytes;
  if (recvmsg(connection, &message, MSG_WAITALL) != sizeof receipt ||
      (message.msg_flags & (MSG_TRUNC | MSG_CTRUNC)) || receipt.magic != MAGIC || (pid_t)receipt.pid != peer || receipt.asid == own_asid()) return 18;
  struct cmsghdr *header = CMSG_FIRSTHDR(&message);
  if (!header || header->cmsg_level != SOL_SOCKET || header->cmsg_type != SCM_RIGHTS || header->cmsg_len != CMSG_LEN(sizeof(int))) return 19;
  int descriptor = -1;
  memcpy(&descriptor, CMSG_DATA(header), sizeof descriptor);
  struct stat metadata;
  unsigned int all = 1;
  unsigned long long drops = 0, reads = 0;
  errno = 0;
  int flags_read = ioctl(descriptor, AUDITSDEV_GET_ALLSESSIONS, &all);
  int flags_error = errno;
  if (fstat(descriptor, &metadata) || !S_ISCHR(metadata.st_mode) ||
      (unsigned long long)metadata.st_rdev != receipt.rdev ||
      ((fcntl(descriptor, F_GETFL) & O_ACCMODE) != O_RDONLY) ||
      (flags_read ? flags_error != EPERM : all != 0) ||
      ioctl(descriptor, AUDITSDEV_GET_DROPS, &drops) || drops) return 20;
  mach_port_t named = MACH_PORT_NULL;
  audit_token_t token = {0}; mach_msg_type_number_t count = TASK_AUDIT_TOKEN_COUNT;
  if (task_name_for_pid(mach_task_self(), peer, &named) || task_info(named, TASK_AUDIT_TOKEN, (task_info_t)&token, &count) || token.val[5] != receipt.pid || token.val[6] != receipt.asid) return 21;
  audit_token_t parent_token = token;
  mach_port_deallocate(mach_task_self(), named);
  if (rust_path) {
    named = MACH_PORT_NULL; count = TASK_AUDIT_TOKEN_COUNT;
    if (receipt.owner_pid == receipt.pid ||
        task_name_for_pid(mach_task_self(), receipt.owner_pid, &named) ||
        task_info(named, TASK_AUDIT_TOKEN, (task_info_t)&parent_token, &count) ||
        parent_token.val[5] != receipt.owner_pid || parent_token.val[6] != receipt.asid) return 46;
    char verified[PROC_PIDPATHINFO_MAXSIZE] = {0};
    if (proc_pidpath_audittoken(&parent_token, verified, sizeof verified) <= 0 || strcmp(verified, rust_path)) return 47;
    mach_port_deallocate(mach_task_self(), named);
  }
  if (write(connection, "A", 1) != 1) return 22;
  pid_t child;
  if (read(connection, &child, sizeof child) != sizeof child) return 23;
  named = MACH_PORT_NULL;
  count = TASK_AUDIT_TOKEN_COUNT;
  if (task_name_for_pid(mach_task_self(), child, &named) ||
      task_info(named, TASK_AUDIT_TOKEN, (task_info_t)&token, &count) ||
      token.val[5] != (unsigned int)child || token.val[6] != receipt.asid) return 29;
  mach_port_deallocate(mach_task_self(), named);
  if (crash) {
    int killed = proc_signal_with_audittoken(&parent_token, SIGKILL);
    printf("producer_crash_signal=%d rust_exporter_crash=%s\n", killed, rust_path ? "true" : "false");
    if (killed) return 33;
    if (rust_path && write(connection, "E", 1) != 1) return 34;
  } else if (write(connection, "E", 1) != 1) return 34;
  close(connection); close(listener); unlink(path);
  struct timespec start;
  if (clock_gettime(CLOCK_MONOTONIC, &start)) return 30;
  printf("observer_asid=%u producer_asid=%u peer_authenticated=true fd_character_device=true all_sessions=0 child_pid=%d\n", own_asid(), receipt.asid, child);
  struct pollfd native_poll = {descriptor, POLLIN, 0};
  int polled = poll(&native_poll, 1, 0);
  printf("device_poll_result=%d revents=%d\n", polled, native_poll.revents);
  fd_set early; FD_ZERO(&early); FD_SET(descriptor, &early);
  struct timeval short_wait = {0, 100000};
  int early_ready = select(descriptor + 1, &early, NULL, NULL, &short_wait);
  printf("no_end_while_authenticated_child_alive=%d\n", early_ready == 0);
  if (early_ready != 0) return 35;
  int end = 0, close_event = 0;
  for (int attempt = 0; attempt < 60 && !close_event; ++attempt) {
    fd_set reading;
    FD_ZERO(&reading); FD_SET(descriptor, &reading);
    struct timeval timeout = {0, 100000};
    int available = select(descriptor + 1, &reading, NULL, NULL, &timeout);
    if (available < 0) return 25;
    if (!available) continue;
    int event = record(descriptor, receipt.asid);
    if (event < 0) { printf("owned_record_decode=failed errno=%d\n", errno); return 26; }
    printf("owned_session_event=%d\n", event);
    if (event == AUE_SESSION_END) {
      struct timespec now;
      if (clock_gettime(CLOCK_MONOTONIC, &now)) return 30;
      long long elapsed = (now.tv_sec - start.tv_sec) * 1000LL + (now.tv_nsec - start.tv_nsec) / 1000000LL;
      printf("session_end_elapsed_ms=%lld child_token_authenticated=true\n", elapsed);
      end = 1;
    }
    if (event == AUE_SESSION_CLOSE) close_event = 1;
  }
  if (ioctl(descriptor, AUDITSDEV_GET_DROPS, &drops) || ioctl(descriptor, AUDITSDEV_GET_READS, &reads)) return 27;
  printf("session_end=%d session_close=%d drops=%llu reads=%llu asid_reuse_retention=false production_containment=false\n", end, close_event, drops, reads);
  close(descriptor);
  return end && drops == 0 ? 0 : 28;
}

static int keeper(const char *service, const char *path) {
  alarm(15);
  unsigned int original_asid = own_asid();
  mach_port_t receive = MACH_PORT_NULL;
  if (bootstrap_check_in(bootstrap_port, service, &receive)) return 36;
  struct sockaddr_un address_value;
  if (address(&address_value, path)) return 37;
  int listener = socket(AF_UNIX, SOCK_STREAM, 0);
  if (listener < 0 || bind(listener, (struct sockaddr *)&address_value, sizeof address_value) || listen(listener, 1)) return 38;
  puts("KEEPER_READY");
  union {
    struct { struct session_message message; mach_msg_audit_trailer_t trailer; } value;
    unsigned char bytes[1024];
  } received;
  memset(&received, 0, sizeof received);
  mach_msg_option_t options = MACH_RCV_MSG | MACH_RCV_TIMEOUT |
    MACH_RCV_TRAILER_TYPE(MACH_MSG_TRAILER_FORMAT_0) |
    MACH_RCV_TRAILER_ELEMENTS(MACH_RCV_TRAILER_AUDIT);
  if (mach_msg((mach_msg_header_t *)received.bytes, options, 0, sizeof received.bytes, receive, 5000, MACH_PORT_NULL)) return 39;
  struct session_message *message = &received.value.message;
  mach_msg_audit_trailer_t *trailer = (mach_msg_audit_trailer_t *)(received.bytes + ((message->header.msgh_size + 3) & ~3));
  if (message->header.msgh_size != sizeof *message || message->header.msgh_id != 6501 ||
      !(message->header.msgh_bits & MACH_MSGH_BITS_COMPLEX) || message->body.msgh_descriptor_count != 1 ||
      message->port.type != MACH_MSG_PORT_DESCRIPTOR || message->magic != MAGIC ||
      trailer->msgh_trailer_size < sizeof *trailer || trailer->msgh_audit.val[5] != message->pid ||
      trailer->msgh_audit.val[6] != message->asid || trailer->msgh_audit.val[1] != getuid() ||
      message->asid == original_asid) return 40;
  char expected_path[PROC_PIDPATHINFO_MAXSIZE] = {0}, peer_path[PROC_PIDPATHINFO_MAXSIZE] = {0};
  if (proc_pidpath(getpid(), expected_path, sizeof expected_path) <= 0 ||
      proc_pidpath_audittoken(&trailer->msgh_audit, peer_path, sizeof peer_path) <= 0 ||
      strcmp(expected_path, peer_path)) return 45;
  /* A received Mach right is never supplied to audit_session_join or a spawn
   * attribute. Its descriptor is retained and generically deallocated only.
   * The authenticated trailer proves the sender, not the right's kernel type. */
  if (own_asid() != original_asid) return 41;
  printf("claimed_target_asid=%u keeper_outside_asid=%u genuine_mach_peer=true received_right_unclassified=true\n",
         message->asid, original_asid);
  int connection = accept(listener, NULL, NULL);
  if (connection < 0) return 42;
  uid_t uid; gid_t gid;
  if (getpeereid(connection, &uid, &gid) || uid != getuid()) return 43;
  char command;
  if (read(connection, &command, 1) != 1 || command != 'D') return 44;
  mach_port_deallocate(mach_task_self(), message->port.name);
  puts("unclassified_received_right_released=true");
  close(connection); close(listener); unlink(path);
  mach_port_deallocate(mach_task_self(), receive);
  return 0;
}

int main(int argc, char **argv) {
  setvbuf(stdout, NULL, _IOLBF, 0);
  if (argc == 3 && !strcmp(argv[1], "observer")) return observer(argv[2], 0, NULL);
  if (argc == 3 && !strcmp(argv[1], "observer-crash")) return observer(argv[2], 1, NULL);
  if ((argc == 3 || argc == 4) && !strcmp(argv[1], "producer")) return producer(argv[2], argc == 4 ? argv[3] : NULL, -1, 0);
  if (argc == 6 && !strcmp(argv[1], "producer-api")) return producer(argv[2], argv[3], atoi(argv[4]), (unsigned int)atoi(argv[5]));
  if (argc == 4 && !strcmp(argv[1], "observer-rust-crash")) return observer(argv[2], 1, argv[3]);
  if (argc == 4 && !strcmp(argv[1], "keeper")) return keeper(argv[2], argv[3]);
  return 2;
}
