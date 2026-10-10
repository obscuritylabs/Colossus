#include "colossus_cef.h"
#include "include/cef_sandbox_win.h"

namespace {
int32_t RejectBrowserMain() { return 1; }
}

// A small CEF-only client DLL avoids loading Tauri, provider, dictation or
// credential libraries into sandboxed renderer/GPU/utility child processes.
CEF_BOOTSTRAP_EXPORT int RunWinMain(HINSTANCE instance, LPWSTR, int,
    void* sandbox, cef_version_info_t* version) {
  return colossus_cef_windows_run(reinterpret_cast<uintptr_t>(instance), sandbox,
                                version, RejectBrowserMain);
}
