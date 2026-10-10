#include "host_internal.h"
#if defined(OS_WIN)
#include <windows.h>
#endif

namespace colossus {
#if !defined(OS_MAC)
bool PlatformEarlySetup() { return true; }
bool PlatformLoadLibrary() { return true; }
void PlatformEventLoopDiagnostics() {}
colossus_cef_bounds PlatformChildBounds(uintptr_t, colossus_cef_bounds bounds) { return bounds; }
bool PlatformAcceptanceEvidence(CefWindowHandle, const void*, size_t, std::string*) { return false; }
#endif
int32_t PlatformBounds(CefWindowHandle handle, colossus_cef_bounds b) {
#if defined(OS_WIN)
  return SetWindowPos(handle, nullptr, b.x, b.y, b.width, b.height,
    SWP_NOACTIVATE | SWP_NOZORDER) ? COLOSSUS_CEF_OK : COLOSSUS_CEF_CLOSED;
#elif defined(OS_LINUX)
  (void)handle; (void)b; return COLOSSUS_CEF_UNAVAILABLE;
#else
  extern int32_t MacBounds(CefWindowHandle, colossus_cef_bounds);
  return MacBounds(handle, b);
#endif
}
int32_t PlatformVisible(CefWindowHandle handle, bool visible) {
#if defined(OS_WIN)
  if (!IsWindow(handle)) return COLOSSUS_CEF_CLOSED;
  ShowWindow(handle, visible ? SW_SHOWNA : SW_HIDE);
  EnableWindow(handle, visible); return COLOSSUS_CEF_OK;
#elif defined(OS_LINUX)
  (void)handle; (void)visible; return COLOSSUS_CEF_UNAVAILABLE;
#else
  extern int32_t MacVisible(CefWindowHandle, bool);
  return MacVisible(handle, visible);
#endif
}
}
