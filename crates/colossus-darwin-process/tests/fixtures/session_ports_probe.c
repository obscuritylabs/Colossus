/* Fixed nonbrowser native proofs. No receiver/keeper joins any foreign session. */
#include <bsm/audit.h>
#include <bsm/audit_session.h>
#include <bsm/libbsm.h>
#include <errno.h>
#include <fcntl.h>
#include <libproc.h>
#include <mach/mach.h>
#include <mach/task_info.h>
#include <security/audit/audit_ioctl.h>
#include <servers/bootstrap.h>
#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/fileport.h>
#include <sys/ioctl.h>
#include <sys/select.h>
#include <sys/stat.h>
#include <unistd.h>

static const unsigned int MAGIC = 0xC01055A6;
struct transfer {
  mach_msg_header_t header;
  mach_msg_body_t body;
  mach_msg_port_descriptor_t session, device;
  unsigned int magic, pid, asid;
  unsigned long long rdev;
};
struct acknowledgment { mach_msg_header_t header; unsigned int magic; };

static int bind_token(pid_t pid, audit_token_t *token) {
  mach_port_t named = MACH_PORT_NULL;
  mach_msg_type_number_t count = TASK_AUDIT_TOKEN_COUNT;
  int valid = !task_name_for_pid(mach_task_self(), pid, &named) &&
    !task_info(named, TASK_AUDIT_TOKEN, (task_info_t)token, &count) &&
    count == TASK_AUDIT_TOKEN_COUNT && token->val[5] == (unsigned int)pid;
  if (named) mach_port_deallocate(mach_task_self(), named);
  return valid ? 0 : -1;
}

static unsigned int own_asid(void) {
  audit_token_t token = {0};
  if (bind_token(getpid(), &token)) return 0;
  return token.val[6];
}

static int unconsumed_device(int descriptor, unsigned long long exact_rdev) {
  struct stat received = {0}, expected = {0};
  unsigned long long drops = 0, reads = 0;
  unsigned int maximum = 0;
  int flags = fcntl(descriptor, F_GETFL), fdflags = fcntl(descriptor, F_GETFD);
  int max_result = ioctl(descriptor, AUDITSDEV_GET_MAXDATA, &maximum);
  int drops_result = ioctl(descriptor, AUDITSDEV_GET_DROPS, &drops);
  int reads_result = ioctl(descriptor, AUDITSDEV_GET_READS, &reads);
  printf("device_preflight max_result=%d maximum=%u fixture_maximum=%u drops_result=%d reads_result=%d drops=%llu reads=%llu fdflags=%d openflags=%d\n",
      max_result, maximum, (unsigned int)MAXAUDITDATA, drops_result, reads_result, drops, reads, fdflags, flags);
  return !fstat(descriptor, &received) && !stat("/dev/auditsessions", &expected) &&
    S_ISCHR(received.st_mode) && major(received.st_rdev) == major(expected.st_rdev) &&
    (!exact_rdev || (unsigned long long)received.st_rdev == exact_rdev) &&
    flags >= 0 && (flags & O_ACCMODE) == O_RDONLY && fdflags >= 0 &&
    (fdflags & FD_CLOEXEC) && !max_result &&
    maximum > 0 && maximum <= MAXAUDITDATA &&
    !drops_result && drops == 0 && !reads_result && reads == 0;
}

/* Received rights are retained and released only. Never classify, join or
 * spawn with a conveyed Mach right: wrong-type audit-session spawn cleanup
 * panics macOS 26.5.2. The exact fixture producer's current-only export is the
 * provenance of this reference; an audit trailer alone does not certify it. */

static int producer(const char *service) {
  alarm(10);
  mach_port_t remote = MACH_PORT_NULL, reply = MACH_PORT_NULL;
  mach_port_t session = audit_session_self();
  int descriptor = open("/dev/auditsessions", O_RDONLY | O_NONBLOCK | O_CLOEXEC);
  if (!session || descriptor < 0 || !unconsumed_device(descriptor, 0)) return 10;
  fileport_t device = FILEPORT_NULL;
  if (fileport_makeport(descriptor, &device)) {
    printf("fileport_makeport=-1 errno=%d\n", errno); return 11;
  }
  if (bootstrap_look_up(bootstrap_port, service, &remote) ||
      mach_port_allocate(mach_task_self(), MACH_PORT_RIGHT_RECEIVE, &reply)) return 12;
  struct transfer message = {0};
  message.header.msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_COPY_SEND, MACH_MSG_TYPE_MAKE_SEND_ONCE) |
    MACH_MSGH_BITS_COMPLEX;
  message.header.msgh_remote_port = remote;
  message.header.msgh_local_port = reply;
  message.header.msgh_size = sizeof message;
  message.header.msgh_id = 6502;
  message.body.msgh_descriptor_count = 2;
  message.session.name = session;
  message.session.disposition = MACH_MSG_TYPE_COPY_SEND;
  message.session.type = MACH_MSG_PORT_DESCRIPTOR;
  message.device.name = device;
  message.device.disposition = MACH_MSG_TYPE_MOVE_SEND;
  message.device.type = MACH_MSG_PORT_DESCRIPTOR;
  message.magic = MAGIC; message.pid = (unsigned int)getpid(); message.asid = own_asid();
  struct stat metadata = {0};
  if (fstat(descriptor, &metadata)) return 10;
  message.rdev = (unsigned long long)metadata.st_rdev;
  kern_return_t sent = mach_msg(&message.header, MACH_SEND_MSG | MACH_SEND_TIMEOUT,
      sizeof message, 0, MACH_PORT_NULL, 5000, MACH_PORT_NULL);
  if (sent) { mach_port_deallocate(mach_task_self(), device); return 13; }
  close(descriptor);
  mach_port_deallocate(mach_task_self(), session);
  mach_port_deallocate(mach_task_self(), remote);
  union { struct acknowledgment message; unsigned char bytes[256]; } response = {0};
  if (mach_msg((mach_msg_header_t *)response.bytes, MACH_RCV_MSG | MACH_RCV_TIMEOUT,
      0, sizeof response.bytes, reply, 5000, MACH_PORT_NULL) ||
      response.message.header.msgh_size != sizeof(struct acknowledgment) ||
      response.message.header.msgh_id != 6503 || response.message.magic != MAGIC) return 14;
  mach_port_mod_refs(mach_task_self(), reply, MACH_PORT_RIGHT_RECEIVE, -1);
  printf("atomic_session_fileport_transfer=true producer_asid=%u\n", own_asid());
  return 0;
}

static int read_exact(int descriptor, unsigned char *bytes, size_t length) {
  size_t used = 0;
  for (int pass = 0; pass < 50 && used < length; ++pass) {
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

static int owned_record(int descriptor, unsigned int expected) {
  unsigned char bytes[MAXAUDITDATA];
  if (read_exact(descriptor, bytes, 5)) return -1;
  unsigned int length = ((unsigned int)bytes[1] << 24) | ((unsigned int)bytes[2] << 16) |
    ((unsigned int)bytes[3] << 8) | bytes[4];
  if (length < 68 || length > sizeof bytes || read_exact(descriptor, bytes + 5, length - 5)) return -1;
  int event = -1, owned = 0, trailer = 0;
  size_t offset = 0;
  while (offset < length) {
    tokenstr_t token;
    if (au_fetch_tok(&token, bytes + offset, (int)(length - offset)) || !token.len ||
        token.len > length - offset) return -1;
    switch (token.id) {
      case AUT_HEADER32: event = token.tt.hdr32.e_type; break;
      case AUT_HEADER32_EX: event = token.tt.hdr32_ex.e_type; break;
      case AUT_HEADER64: event = token.tt.hdr64.e_type; break;
      case AUT_HEADER64_EX: event = token.tt.hdr64_ex.e_type; break;
      case AUT_SUBJECT32: owned = token.tt.subj32.sid == expected; break;
      case AUT_SUBJECT32_EX: owned = token.tt.subj32_ex.sid == expected; break;
      case AUT_SUBJECT64: owned = token.tt.subj64.sid == expected; break;
      case AUT_SUBJECT64_EX: owned = token.tt.subj64_ex.sid == expected; break;
      case AUT_TRAILER: trailer = offset + token.len == length; break;
      default: break;
    }
    offset += token.len;
  }
  return owned && trailer && event >= AUE_SESSION_START && event <= AUE_SESSION_CLOSE ? event : -1;
}

static int receiver(const char *service) {
  alarm(15);
  unsigned int before = own_asid();
  mach_port_t original = audit_session_self(), receive = MACH_PORT_NULL;
  if (!before || !original || bootstrap_check_in(bootstrap_port, service, &receive)) return 20;
  puts("RECEIVER_READY");
  union { struct transfer message; unsigned char bytes[1024]; } received = {0};
  mach_msg_option_t options = MACH_RCV_MSG | MACH_RCV_TIMEOUT |
    MACH_RCV_TRAILER_TYPE(MACH_MSG_TRAILER_FORMAT_0) |
    MACH_RCV_TRAILER_ELEMENTS(MACH_RCV_TRAILER_AUDIT);
  if (mach_msg((mach_msg_header_t *)received.bytes, options, 0, sizeof received.bytes,
      receive, 5000, MACH_PORT_NULL)) return 21;
  struct transfer *message = &received.message;
  mach_msg_audit_trailer_t *trailer = (mach_msg_audit_trailer_t *)(received.bytes +
      ((message->header.msgh_size + 3) & ~3));
  if (message->header.msgh_size != sizeof *message || message->header.msgh_id != 6502 ||
      !(message->header.msgh_bits & MACH_MSGH_BITS_COMPLEX) || message->body.msgh_descriptor_count != 2 ||
      message->session.type != MACH_MSG_PORT_DESCRIPTOR || message->device.type != MACH_MSG_PORT_DESCRIPTOR ||
      message->session.disposition != MACH_MSG_TYPE_PORT_SEND || message->device.disposition != MACH_MSG_TYPE_PORT_SEND ||
      message->magic != MAGIC || trailer->msgh_trailer_size < sizeof *trailer ||
      trailer->msgh_audit.val[5] != message->pid || trailer->msgh_audit.val[6] != message->asid ||
      trailer->msgh_audit.val[1] != getuid() || trailer->msgh_audit.val[3] != getuid() ||
      message->asid == before) return 22;
  char expected_path[PROC_PIDPATHINFO_MAXSIZE] = {0}, peer_path[PROC_PIDPATHINFO_MAXSIZE] = {0};
  if (proc_pidpath(getpid(), expected_path, sizeof expected_path) <= 0 ||
      proc_pidpath_audittoken(&trailer->msgh_audit, peer_path, sizeof peer_path) <= 0 ||
      strcmp(expected_path, peer_path)) return 23;
  int descriptor = fileport_makefd(message->device.name);
  if (descriptor < 0 || !unconsumed_device(descriptor, message->rdev)) return 24;
  mach_port_deallocate(mach_task_self(), message->device.name);
  puts("fileport_makefd=true exact_readonly_char_device=true cloexec=true zero_reads_and_drops=true genuine_mach_peer=true");
  if (own_asid() != before) return 25;
  struct acknowledgment acknowledgment = {0};
  acknowledgment.header.msgh_bits = MACH_MSGH_BITS(MACH_MSG_TYPE_MOVE_SEND_ONCE, 0);
  acknowledgment.header.msgh_remote_port = message->header.msgh_remote_port;
  acknowledgment.header.msgh_size = sizeof acknowledgment;
  acknowledgment.header.msgh_id = 6503; acknowledgment.magic = MAGIC;
  if (mach_msg(&acknowledgment.header, MACH_SEND_MSG | MACH_SEND_TIMEOUT,
      sizeof acknowledgment, 0, MACH_PORT_NULL, 5000, MACH_PORT_NULL)) return 26;
  int end = 0;
  for (int attempt = 0; attempt < 50 && !end; ++attempt) {
    fd_set ready; FD_ZERO(&ready); FD_SET(descriptor, &ready);
    struct timeval timeout = {0, 100000};
    int available = select(descriptor + 1, &ready, NULL, NULL, &timeout);
    if (available < 0) return 27;
    if (!available) continue;
    int event = owned_record(descriptor, message->asid);
    if (event < 0) return 28;
    end = event == AUE_SESSION_END;
  }
  unsigned long long drops = 0;
  if (!end || ioctl(descriptor, AUDITSDEV_GET_DROPS, &drops) || drops || own_asid() != before) return 29;
  printf("kernel_session_end=true target_asid=%u keeper_asid=%u keeper_never_joined=true drops=%llu retained_through_end=true\n",
      message->asid, before, drops);
  close(descriptor);
  mach_port_deallocate(mach_task_self(), message->session.name);
  mach_port_deallocate(mach_task_self(), original);
  mach_port_deallocate(mach_task_self(), receive);
  return 0;
}

int main(int argc, char **argv) {
  setvbuf(stdout, NULL, _IOLBF, 0);
  if (argc == 3 && !strcmp(argv[1], "marker")) {
    int marker = open(argv[2], O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, 0600);
    if (marker < 0 || write(marker, "M", 1) != 1 || close(marker)) return 30;
    printf("MARKER_REACHED asid=%u\n", own_asid());
    return 0;
  }
  if (argc == 3 && !strcmp(argv[1], "producer")) return producer(argv[2]);
  if (argc == 3 && !strcmp(argv[1], "receiver")) return receiver(argv[2]);
  return 2;
}
