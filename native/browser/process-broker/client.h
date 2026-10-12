#ifndef COLOSSUS_OWNED_PROCESS_CLIENT_H_
#define COLOSSUS_OWNED_PROCESS_CLIENT_H_

#include <stdint.h>
#include <sys/types.h>
#include <string>
#include <vector>
#include <chrono>

#if __has_include("base/base_export.h")
#include "base/base_export.h"
#define COLOSSUS_OWNED_PROCESS_EXPORT BASE_EXPORT
#else
#define COLOSSUS_OWNED_PROCESS_EXPORT __attribute__((visibility("default")))
#endif

// The native C loader ABI must be visible even in CEF's non-component build,
// where BASE_EXPORT intentionally expands to nothing and base uses hidden
// visibility. These two functions are the complete private dynamic ABI.
#define COLOSSUS_OWNED_LOADER_EXPORT __attribute__((visibility("default")))

// Private native-loader ABI. Install once from a broker-created channel before
// loading CEF. No URL, model argument, environment variable or browser JS calls
// this API. The native loader validates the sealed bootstrap and signed peers.
extern "C" COLOSSUS_OWNED_LOADER_EXPORT bool colossus_owned_process_install(int fd,
                                               const uint8_t nonce[16],
                                               pid_t logical_browser_pid);
extern "C" COLOSSUS_OWNED_LOADER_EXPORT bool colossus_owned_helper_install(pid_t logical_browser_pid);

namespace colossus_owned_process {
COLOSSUS_OWNED_PROCESS_EXPORT bool Enabled();
COLOSSUS_OWNED_PROCESS_EXPORT pid_t LogicalBrowserPid();
// Args exclude argv[0]. The broker selects the fixed executable from the role.
COLOSSUS_OWNED_PROCESS_EXPORT pid_t SpawnStopped(const std::vector<std::string>& args);
COLOSSUS_OWNED_PROCESS_EXPORT bool Resume(pid_t pid);
COLOSSUS_OWNED_PROCESS_EXPORT bool Status(pid_t pid, bool* exited, int* raw_wait_status);
COLOSSUS_OWNED_PROCESS_EXPORT bool Terminate(pid_t pid);
COLOSSUS_OWNED_PROCESS_EXPORT bool WaitForExit(pid_t pid,std::chrono::milliseconds timeout,int* raw_wait_status);
}  // namespace colossus_owned_process

#endif
