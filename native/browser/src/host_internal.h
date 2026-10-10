#ifndef COLOSSUS_CEF_HOST_INTERNAL_H
#define COLOSSUS_CEF_HOST_INTERNAL_H

#include "colossus_cef.h"
#include "include/cef_browser.h"
#include "include/cef_client.h"
#include "include/cef_devtools_message_observer.h"
#include "include/cef_download_handler.h"
#include "include/cef_parser.h"
#include "include/cef_request_context.h"
#include "include/cef_request_context_handler.h"
#include "include/cef_render_handler.h"
#include <map>
#include <string>
#include <chrono>
#include <vector>

namespace colossus {
struct Tab {
  uint64_t generation;
  uint64_t context_id;
  CefRefPtr<CefBrowser> browser;
  CefRefPtr<CefClient> client;
  CefRefPtr<CefRegistration> observer_registration;
  bool closing = false;
  bool visible = false;
  bool acceptance_probe_pending = false;
  uint64_t identity_request = 0;
  CefRefPtr<CefSelectClientCertificateCallback> identity_callback;
  std::vector<CefRefPtr<CefX509Certificate>> identity_candidates;
  std::chrono::steady_clock::time_point identity_deadline;
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
void CancelIdentity(colossus_cef_tab tab, Tab& state);
bool DownloadCan(colossus_cef_tab tab, uint64_t generation, const std::string& url,
                 const std::string& method);
bool DownloadBefore(colossus_cef_tab tab, uint64_t generation,
                    CefRefPtr<CefDownloadItem> item, CefRefPtr<CefBeforeDownloadCallback> callback);
void DownloadUpdated(colossus_cef_tab tab, uint64_t generation,
                     CefRefPtr<CefDownloadItem> item, CefRefPtr<CefDownloadItemCallback> callback);
void DownloadExpire();
void DownloadRedirectDenied(colossus_cef_tab tab, uint64_t generation);
void DownloadClosed(colossus_cef_tab tab);
bool ProfileRoot(const char* root);
int32_t ProfileContext(uint64_t owner, std::string* cache);
void ProfileShutdown();
bool PresentationRect(colossus_cef_tab tab, uint64_t generation, CefRect* rect, double* scale);
bool PresentationFocused(colossus_cef_tab tab, uint64_t generation);
void PresentationPaint(colossus_cef_tab tab, uint64_t generation,
  CefRenderHandler::PaintElementType kind, const void* pixels, int width, int height);
void PresentationExpire();
void PresentationDocument(colossus_cef_tab tab, uint64_t generation);
void PresentationClosed(colossus_cef_tab tab);
int32_t PlatformBounds(CefWindowHandle handle, colossus_cef_bounds bounds);
int32_t PlatformVisible(CefWindowHandle handle, bool visible);
bool PlatformEarlySetup();
bool PlatformLoadLibrary();
bool OnOwningThread();
void PlatformEventLoopDiagnostics();
bool PlatformCloseChild(CefWindowHandle handle);
int32_t PlatformAcceptanceActivate(CefWindowHandle handle);
int32_t PlatformAcceptanceTerminate(CefWindowHandle handle);
#if defined(OS_WIN)
int32_t PlatformAcceptanceInput(CefWindowHandle handle);
#endif
colossus_cef_bounds PlatformChildBounds(uintptr_t parent, colossus_cef_bounds bounds);
bool ValidBounds(colossus_cef_bounds bounds);
bool ProxyCredentials(const std::string& host, int port,
  std::string* username, std::string* password);
bool PlatformAcceptanceEvidence(CefWindowHandle handle, const void* result,
                                size_t size, std::string* evidence);
}
#endif
