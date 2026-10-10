#include "colossus_cef.h"
#include "include/base/cef_logging.h"
#include "include/cef_app.h"
#include "include/cef_version_info.h"
#include "include/wrapper/cef_library_loader.h"

#include <windows.h>
#include <cstring>
#include <filesystem>
#include <atomic>
#include <mutex>
#include <thread>

namespace {
uintptr_t entry_instance = 0;
void* entry_sandbox = nullptr;
std::thread::id entry_thread;
std::mutex entry_mutex;
std::atomic_bool entered{false};

bool VersionMatches(const cef_version_info_t* supplied) {
  if (!supplied || supplied->size < CEF_VERSION_INFO_SIZE_WITH_SANDBOX_HASH)
    return false;
  cef_version_info_t expected{};
  CEF_POPULATE_VERSION_INFO(&expected);
  return supplied->cef_version_major == expected.cef_version_major &&
         supplied->cef_version_minor == expected.cef_version_minor &&
         supplied->cef_version_patch == expected.cef_version_patch &&
         supplied->cef_commit_number == expected.cef_commit_number &&
         supplied->chrome_version_major == expected.chrome_version_major &&
         supplied->chrome_version_minor == expected.chrome_version_minor &&
         supplied->chrome_version_build == expected.chrome_version_build &&
         supplied->chrome_version_patch == expected.chrome_version_patch &&
         std::memcmp(supplied->sandbox_compat_hash, expected.sandbox_compat_hash,
                     sizeof(expected.sandbox_compat_hash)) == 0;
}
}

// This is reached only through the client DLL's RunWinMain, called by the pinned
// CEF bootstrap. The loader and genuine sandbox context span the entire Rust
// Desktop lifecycle. Sub-processes execute before Tauri, Tokio or private homes.
extern "C" int32_t colossus_cef_windows_run(uintptr_t instance, void* sandbox,
    void* supplied_version, int32_t (*run)()) {
  auto* version = static_cast<cef_version_info_t*>(supplied_version);
  if (!instance || !sandbox || !run || !VersionMatches(version)) return 1;
  bool expected = false;
  if (!entered.compare_exchange_strong(expected, true)) return 1;
  CefScopedLibraryLoader loader;
  {
    cef::logging::ScopedEarlySupport logging({});
    if (!loader.LoadInSubProcessAssert(version)) {
      // Installed auto-download/installer paths are deliberately ignored. This
      // developer entry accepts only the adjacent, inventoried pinned engine.
      wchar_t executable[32768]{};
      const DWORD length = GetModuleFileNameW(nullptr, executable, 32768);
      if (!length || length >= 32768) return 1;
      const auto path = std::filesystem::path(executable).parent_path() / L"libcef.dll";
      // Unsigned bytes are permitted only in this Rust debug-only preview lane.
      // The CEF bootstrap separately verifies EXE/client DLL signature parity.
      if (!loader.LoadInMainAssert(path.c_str(), nullptr, true, version)) return 1;
    }
  }
  const int subprocess = CefExecuteProcess(
      CefMainArgs(reinterpret_cast<HINSTANCE>(instance)), nullptr, sandbox);
  if (subprocess >= 0) return subprocess;
  {
    std::lock_guard lock(entry_mutex);
    entry_instance = instance;
    entry_sandbox = sandbox;
    entry_thread = std::this_thread::get_id();
  }
  const int result = run();
  {
    std::lock_guard lock(entry_mutex);
    entry_instance = 0;
    entry_sandbox = nullptr;
  }
  return result;
}

extern "C" int32_t colossus_cef_windows_context(uintptr_t* instance, void** sandbox) {
  std::lock_guard lock(entry_mutex);
  if (!instance || !sandbox || !entry_instance || !entry_sandbox ||
      entry_thread != std::this_thread::get_id()) return COLOSSUS_CEF_UNAVAILABLE;
  *instance = entry_instance;
  *sandbox = entry_sandbox;
  return COLOSSUS_CEF_OK;
}
