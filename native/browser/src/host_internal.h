#ifndef COLOSSUS_CEF_HOST_INTERNAL_H
#define COLOSSUS_CEF_HOST_INTERNAL_H

#include "colossus_cef.h"
#include "include/cef_browser.h"
#include "include/cef_client.h"
#include "include/cef_devtools_message_observer.h"
#include "include/cef_parser.h"
#include "include/cef_request_context.h"
#include "include/cef_request_context_handler.h"
#include <map>
#include <string>

namespace colossus {
struct Tab {
  uint64_t generation;
  uint64_t context_id;
  CefRefPtr<CefBrowser> browser;
  CefRefPtr<CefClient> client;
  CefRefPtr<CefRegistration> observer_registration;
  bool closing = false;
  bool visible = false;
};
extern colossus_cef_callbacks callbacks;
extern std::map<colossus_cef_tab, Tab> tabs;
extern std::map<uint64_t, CefRefPtr<CefRequestContext>> contexts;
extern bool initialized;
extern bool headless;
void Emit(colossus_cef_tab tab, uint64_t generation, uint32_t event,
          int32_t command = 0, int32_t success = 1,
          const void* data = nullptr, size_t size = 0);
bool Allow(colossus_cef_tab tab, uint64_t generation, const std::string& url,
           bool navigation);
void State(colossus_cef_tab tab);
CefRefPtr<CefClient> MakeClient(colossus_cef_tab tab, uint64_t generation,
                              colossus_cef_bounds bounds);
CefRefPtr<CefRequestContextHandler> MakeContextBoundary();
int32_t Resolve(colossus_cef_tab tab, uint64_t generation, Tab** out);
int32_t PlatformBounds(CefWindowHandle handle, colossus_cef_bounds bounds);
int32_t PlatformVisible(CefWindowHandle handle, bool visible);
bool PlatformEarlySetup();
bool PlatformLoadLibrary();
colossus_cef_bounds PlatformChildBounds(uintptr_t parent, colossus_cef_bounds bounds);
bool ValidBounds(colossus_cef_bounds bounds);
}
#endif
