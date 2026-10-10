#ifndef COLOSSUS_CEF_H
#define COLOSSUS_CEF_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define COLOSSUS_CEF_ABI_VERSION 2u
#define COLOSSUS_CEF_MAX_PROTOCOL_BYTES (16u * 1024u * 1024u)

/* Native-only ABI. Never expose native parents, CDP or these callbacks to web IPC.
 * Call all functions on the CEF UI/main thread, except bootstrap (before any UI).
 * Callback strings/bytes are borrowed for the duration of the callback only.
 * No callback may unwind across this boundary. The owner must outlive shutdown.
 */
typedef uint64_t colossus_cef_tab;
enum colossus_cef_status {
  COLOSSUS_CEF_OK = 0,
  COLOSSUS_CEF_INVALID = 1,
  COLOSSUS_CEF_UNAVAILABLE = 2,
  COLOSSUS_CEF_CLOSED = 3,
  COLOSSUS_CEF_WRONG_THREAD = 4,
  COLOSSUS_CEF_DENIED = 5,
  COLOSSUS_CEF_BUSY = 6
};
enum colossus_cef_event {
  COLOSSUS_CEF_CREATED = 1,
  COLOSSUS_CEF_LOADING = 2,
  COLOSSUS_CEF_STATE = 3,
  COLOSSUS_CEF_FAILED = 4,
  COLOSSUS_CEF_CRASHED = 5,
  COLOSSUS_CEF_BLOCKED = 6,
  COLOSSUS_CEF_DOWNLOAD_BLOCKED = 7,
  COLOSSUS_CEF_POPUP_BLOCKED = 8,
  COLOSSUS_CEF_TLS_FAILED = 9,
  COLOSSUS_CEF_CLOSED_EVENT = 10,
  COLOSSUS_CEF_DEVTOOLS_RESULT = 11,
  COLOSSUS_CEF_DEVTOOLS_EVENT = 12,
  COLOSSUS_CEF_DEVTOOLS_DETACHED = 13,
  COLOSSUS_CEF_PKI_SELECTION_REQUIRED = 14,
  COLOSSUS_CEF_PROTOCOL_OVERFLOW = 15,
  COLOSSUS_CEF_ACCEPTANCE_EVIDENCE = 16,
  /* Trusted native AppKit Quit request, never a page/tab event. */
  COLOSSUS_CEF_APPLICATION_QUIT = 17,
  COLOSSUS_CEF_PKI_SELECTION_CANCELLED = 18
};
enum colossus_cef_navigation { COLOSSUS_CEF_BACK = 1, COLOSSUS_CEF_FORWARD = 2,
  COLOSSUS_CEF_RELOAD = 3, COLOSSUS_CEF_STOP = 4 };
typedef struct colossus_cef_bounds { int32_t x, y, width, height; } colossus_cef_bounds;
typedef struct colossus_cef_certificate { const uint8_t* der; size_t der_len; }
  colossus_cef_certificate;
typedef void (*colossus_cef_event_fn)(void* owner, colossus_cef_tab tab,
  uint64_t generation, uint32_t event, int32_t command_id, int32_t success,
  const uint8_t* payload, size_t payload_len);
/* Called on UI and IO threads. Must be thread-safe, bounded and non-blocking.
 * navigation=1 is document navigation; 0 is a resource request. This callback
 * supplements network containment, and does not enforce WebSocket/WebRTC egress.
 */
typedef int32_t (*colossus_cef_allow_url_fn)(void* owner, colossus_cef_tab tab,
  uint64_t generation, const char* url, size_t url_len, int32_t navigation);
/* Native provisioning must match an exact HTTPS origin and reviewed certificate
 * fingerprint. Return a candidate index, -1 (no identity), or -2 to defer native
 * review. Deferred requests expire after 120 seconds and cancel on navigation,
 * stop, close, or replacement. Complete only the exact returned request_id.
 * CEF accesses the platform private key itself; no key crosses this ABI.
 */
typedef int32_t (*colossus_cef_select_identity_fn)(void* owner,
  colossus_cef_tab tab, uint64_t generation, uint64_t request_id,
  const char* origin, size_t origin_len,
  const colossus_cef_certificate* candidates, size_t candidate_count);
typedef struct colossus_cef_callbacks {
  void* owner;
  colossus_cef_event_fn event;
  colossus_cef_allow_url_fn allow_url;
  colossus_cef_select_identity_fn select_identity;
  /* Invoked on any thread; schedule pump on main/UI thread after bounded delay. */
  void (*schedule_pump)(void* owner, int64_t delay_ms);
} colossus_cef_callbacks;
typedef struct colossus_cef_bootstrap_options {
  uint32_t abi_version;
  int32_t argc;
  char** argv;
  /* Windows instance from supported CEF bootstrap client entry. */
  uintptr_t platform_instance;
  /* Required Windows CefExecuteProcess/CefInitialize sandbox bootstrap value. */
  void* sandbox_info;
  const char* root_cache_path;
  const char* browser_subprocess_path;
  /* 0=native child; 1=Linux no-display Ozone; 2=offscreen presentation under
   * supported native bootstrap/application. No release support implied.
   */
  int32_t headless;
  colossus_cef_callbacks callbacks;
} colossus_cef_bootstrap_options;

/* subprocess_exit is >=0 for helper dispatch; -1 for initialized browser host.
 * Returns unavailable if required sandbox/bootstrap/application setup is absent.
 */
int32_t colossus_cef_bootstrap(const colossus_cef_bootstrap_options* options,
                             int32_t* subprocess_exit);
int32_t colossus_cef_pump(void);
#ifdef __APPLE__
/* Dedicated supervised macOS host only. Bounded main-thread AppKit dispatch;
 * Desktop retains its existing Tauri event loop and never calls this function. */
int32_t colossus_cef_standalone_platform_pump(void);
#endif
int32_t colossus_cef_shutdown(void);
#ifdef _WIN32
/* Debug Desktop client-DLL entry. Pass only the pinned bootstrap's borrowed
 * HINSTANCE, sandbox_info and cef_version_info_t. The callback and scoped loader
 * span the whole Desktop lifecycle; CEF subprocesses never invoke the callback.
 */
int32_t colossus_cef_windows_run(uintptr_t instance, void* sandbox_info,
                               void* version_info, int32_t (*run)(void));
/* Returns the borrowed context only on that live entry's owning main thread. */
int32_t colossus_cef_windows_context(uintptr_t* instance, void** sandbox_info);
#endif
/* Private supervisor-only fixed proxy configuration before bootstrap. Numeric
 * address only; credentials authenticate this exact proxy challenge and are never
 * sent to websites. Settings supplement, and do not replace, OS network containment.
 */
int32_t colossus_cef_proxy_configure(const char* address, uint16_t port,
  const char* username, const char* password);
/* Trusted native store lease only; configure before bootstrap. Persistent mode
 * uses the fixed owned root_cache_path/context directory and permits one request
 * context owner for the entire process. Temporary mode retains in-memory stores.
 * No model/renderer pathname or personal-profile discovery is accepted here.
 */
int32_t colossus_cef_profile_persistent(int32_t enabled);
/* parent is NSView* (macOS), HWND (Windows); zero only for headless probe.
 * request_context is isolated in-memory; personal profiles are never attached.
 */
int32_t colossus_cef_create(colossus_cef_tab tab, uint64_t generation, uint64_t context_id,
  uintptr_t parent, colossus_cef_bounds bounds, const char* initial_url);
int32_t colossus_cef_navigate(colossus_cef_tab tab, uint64_t generation,
                             const char* url);
int32_t colossus_cef_control(colossus_cef_tab tab, uint64_t generation,
                            uint32_t navigation);
int32_t colossus_cef_inspect(colossus_cef_tab tab, uint64_t generation);
int32_t colossus_cef_bounds_set(colossus_cef_tab tab, uint64_t generation,
                               colossus_cef_bounds bounds);
int32_t colossus_cef_visible(colossus_cef_tab tab, uint64_t generation,
                            int32_t visible);
int32_t colossus_cef_focus(colossus_cef_tab tab, uint64_t generation);
int32_t colossus_cef_close(colossus_cef_tab tab, uint64_t generation);
/* Native-reviewed selection only. Index -1 cancels. No private key crosses ABI. */
int32_t colossus_cef_select_identity(colossus_cef_tab tab, uint64_t generation,
  uint64_t request_id, int32_t candidate_index);
/* Private adapter-only method; caller must already have consumed runtime permit.
 * Does not open remote-debugging TCP or grant authority. Params are JSON object.
 * Result/event callback is bounded and remains quarantined by caller.
 */
int32_t colossus_cef_devtools(colossus_cef_tab tab, uint64_t generation,
  int32_t command_id, const char* method, const uint8_t* params, size_t params_len);
/* Native development acceptance only: fixed screenshot and NSView/HWND presentation
 * evidence. No page script or caller-supplied DevTools method is accepted.
 * Never expose this entry to ordinary web IPC or production automation.
 */
int32_t colossus_cef_acceptance_probe(colossus_cef_tab tab, uint64_t generation);
/* Test-only request to activate the exact guest's owning application/window.
 * A successful request is not evidence of OS activation; verify the native
 * window's key/active state separately before granting a presentation lease.
 */
int32_t colossus_cef_acceptance_activate(colossus_cef_tab tab, uint64_t generation);
/* Test-only native application close request with a live guest. */
int32_t colossus_cef_acceptance_terminate(colossus_cef_tab tab, uint64_t generation);
#ifdef _WIN32
/* Fixed OS mouse/keyboard fixture input; no caller-selected bytes or coordinates. */
int32_t colossus_cef_acceptance_input(colossus_cef_tab tab, uint64_t generation);
#endif

#ifdef __cplusplus
}
#endif
#endif
