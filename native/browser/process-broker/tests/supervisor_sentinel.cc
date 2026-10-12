// Operator-only supervisor fixture body. No CEF, Cocoa, URLs, credentials,
// environment inspection, local file selectors or descendant creation.
#include "../client.h"

#include <array>
#include <cerrno>
#include <chrono>
#include <cstdint>
#include <poll.h>
#include <sys/types.h>
#include <unistd.h>
#include <vector>

namespace {
bool ExactWrite(int fd, const void* bytes, size_t size) {
  const auto* cursor = static_cast<const uint8_t*>(bytes);
  while (size) {
    const ssize_t count = write(fd, cursor, size);
    if (count < 0 && errno == EINTR) continue;
    if (count <= 0) return false;
    cursor += count;
    size -= static_cast<size_t>(count);
  }
  return true;
}

int Helper(uint8_t role) {
  // The creator consumes these bytes through its actual retained output drains.
  // This constant diagnostic is deliberately not an independent exit receipt.
  const std::array<uint8_t, 9> ready{'C', 'S', 'B', 'O', 'D', 'Y', '0', '1', role};
  if (!ExactWrite(STDOUT_FILENO, ready.data(), ready.size())) return 82;
  // A finite failsafe cannot turn a missing creator/keeper cleanup into a pass:
  // the external acceptance deadline is strictly shorter than this failsafe.
  sleep(60);
  return 83;
}
}  // namespace

extern "C" __attribute__((visibility("default"))) int colossus_owned_body_main(
    int argc, char** argv, uint32_t logical_browser_pid,
    const uint8_t nonce[16], uint8_t role) {
  (void)argv;
  if (role > 4 || logical_browser_pid <= 1 || !nonce) return 80;
  if (role != 0) return Helper(role);
  if (argc != 5 || logical_browser_pid != static_cast<uint32_t>(getpid()) ||
      !colossus_owned_process_install(7, nonce, getpid())) return 80;

  const std::array<std::vector<std::string>, 4> arguments{
      std::vector<std::string>{"--type=renderer", "--shared-files="},
      std::vector<std::string>{"--type=gpu-process", "--shared-files="},
      std::vector<std::string>{"--type=utility", "--service-sandbox-type=network", "--shared-files="},
      std::vector<std::string>{"--type=utility", "--service-sandbox-type=utility", "--shared-files="}};
  std::array<pid_t, 4> children{};
  for (size_t index = 0; index < children.size(); ++index) {
    children[index] = colossus_owned_process::SpawnStopped(arguments[index]);
    if (children[index] <= 1 || !colossus_owned_process::Resume(children[index])) return 81;
  }
  // Resume includes the exact pre-body denial receipt/ACK, but cannot claim that
  // dlopen has completed. Require all four known children to remain live across
  // a bounded settling interval; no raw PID authorizes signalling or adoption.
  const auto settled = std::chrono::steady_clock::now() + std::chrono::milliseconds(300);
  while (std::chrono::steady_clock::now() < settled) {
    for (const pid_t child : children) {
      bool exited = true;
      int status = 0;
      if (!colossus_owned_process::Status(child, &exited, &status) || exited) return 81;
    }
    usleep(10000);
  }
  const std::array<uint8_t, 12> ready{'C', 'S', 'E', 'N', 'T', '0', '0', '1', 1, 2, 3, 4};
  if (!ExactWrite(3, ready.data(), ready.size())) return 82;
  // Main waits on the same opaque operation channel used by the real owner.
  // The fixture sends no page/input operation, and accepts no arbitrary body
  // command. Native creator EOF fencing must reap it before this failsafe.
  pollfd operation{4, POLLIN | POLLHUP, 0};
  const int count = poll(&operation, 1, 60000);
  if (count < 0 && errno == EINTR) return 84;
  return count > 0 ? 0 : 83;
}
