#include "bootstrap_network.h"
#include "host_internal.h"
#include "synthetic_profile.h"
#include "broker_tls.h"
#include "include/cef_app.h"
#include "include/cef_command_line.h"
#include "include/cef_path_util.h"
#include "include/wrapper/cef_helpers.h"
#include <algorithm>
#include <cstdio>
#include <cstring>
#include <filesystem>
#include <thread>
#if defined(OS_MAC)
#include "mac_helper_programs.h"
#endif
#if defined(COLOSSUS_CEF_TEST_CUSTODY)
#include "custody_linux.h"
#endif

namespace colossus {
namespace {
void BootstrapDiagnostic(const char* stage, int32_t status, int32_t detail = 0) {
#if defined(COLOSSUS_CEF_MAC_SYNTHETIC_PROFILE_DIAGNOSTIC)
  // Fixed categories/numbers only. No path, credential, page or policy bytes.
  std::fprintf(stderr, "native cef bootstrap substage=%s status=%d detail=%d\n",
               stage, status, detail);
#else
  (void)stage; (void)status; (void)detail;
#endif
}
}
colossus_cef_callbacks callbacks{};
std::map<colossus_cef_tab, Tab> tabs;
std::map<uint64_t, CefRefPtr<CefRequestContext>> contexts;
bool initialized = false;
bool headless = false;
static std::thread::id owning_thread;
bool OnOwningThread() {
  return owning_thread == std::this_thread::get_id();
}
struct Proxy {
  std::string address;
  uint16_t port = 0;
  std::string username;
  std::string password;
};
static Proxy proxy;
static BrokerTlsTrust broker_tls;

bool BrokerTlsCertificate(const std::string& url, CefRefPtr<CefSSLInfo> info) {
#if defined(OS_MAC) && defined(COLOSSUS_CEF_DEVELOPER_PROCESS_BROKER)
  if (!initialized || !OnOwningThread() || !info ||
      info->GetCertStatus() != CERT_STATUS_AUTHORITY_INVALID) return false;
  CefURLParts parts;
  if (!CefParseURL(url, parts) || CefString(&parts.scheme).ToString() != "https" ||
      parts.username.length || parts.password.length) return false;
  const auto certificate = info->GetX509Certificate();
  const auto der = certificate ? certificate->GetDEREncoded() : nullptr;
  if (!der || !der->GetSize() || der->GetSize() > 65536) return false;
  std::vector<uint8_t> leaf(der->GetSize());
  if (der->GetData(leaf.data(), leaf.size(), 0) != leaf.size()) return false;
  const auto origin = CefString(&parts.origin).ToString();
  return !origin.empty() && broker_tls.Matches(origin.substr(0, origin.size() - 1), leaf.data(),
                           leaf.size(), std::chrono::steady_clock::now());
#else
  (void)url; (void)info;
  return false;
#endif
}

bool ProxyCredentials(const std::string& host, int port,
                      std::string* username, std::string* password) {
  // Immutable after bootstrap; called on CEF IO threads.
  if (!proxy.port || host != proxy.address || port != proxy.port) return false;
  *username = proxy.username; *password = proxy.password; return true;
}

class App final : public CefApp, public CefBrowserProcessHandler {
 public:
  explicit App(bool no_display) : no_display_(no_display) {}
#if defined(OS_MAC)
  App(bool no_display, MacHelperPrograms helpers)
    : no_display_(no_display), owned_helpers_(std::move(helpers)) {}
#endif
  CefRefPtr<CefBrowserProcessHandler> GetBrowserProcessHandler() override { return this; }
#if defined(COLOSSUS_CEF_TEST_CUSTODY)
  CefRefPtr<CefRenderProcessHandler> GetRenderProcessHandler() override { return custody_renderer_; }
#endif
#if defined(COLOSSUS_CEF_TEST_CUSTODY) || defined(OS_MAC)
  void OnBeforeChildProcessLaunch(CefRefPtr<CefCommandLine> line) override {
#if defined(COLOSSUS_CEF_TEST_CUSTODY)
    CustodyChild(line);
#endif
#if defined(OS_MAC)
    if (!owned_helpers_) return;
    const auto process = line->GetSwitchValue("type").ToString();
    const auto program = line->GetProgram().ToString();
    // Chrome's helper discovery assumes its versioned framework layout. This
    // owned component supplies fixed separately validated fronts instead; the
    // creator still selects and verifies the signed image/body for every role.
    const auto selected = owned_helpers_->Program(process);
#if defined(COLOSSUS_CEF_MAC_SYNTHETIC_PROFILE_DIAGNOSTIC)
    std::fprintf(stderr, "native helper launch process=%d program=%d selected=%d\n",
      MacChildProcessClass(process), MacHelperProgramKind(program),
      selected ? MacHelperProgramKind(*selected) : MacHelperProgramKind(program));
#endif
    // CEF's public callback admits only this validated fixed GPU front. Do not
    // turn the generic utility front into a GPU entry or change sandbox flags.
    if (selected) line->SetProgram(*selected);
#endif
  }
#endif
  void OnBeforeCommandLineProcessing(const CefString& process,
                                    CefRefPtr<CefCommandLine> line) override {
    // The exact pinned Chromium testing provider avoids eager OSCrypt Keychain
    // calls for the separately admitted synthetic fixture. Never select it in a
    // helper, from process arguments, or in an ordinary/release native component.
    if (const auto* selection = SyntheticProfileSwitch(process.empty()))
      line->AppendSwitch(selection);
    if (process.empty()) {
      if (no_display_) {
        line->AppendSwitchWithValue("ozone-platform", "headless");
        line->AppendSwitch("disable-gpu");
      }
      line->AppendSwitch("disable-quic");
      line->AppendSwitch("disable-extensions");
      line->AppendSwitch("disable-component-extensions-with-background-pages");
      line->AppendSwitch("disable-component-update");
      line->AppendSwitch("disable-background-networking");
      // Route HTTP/proxy authentication to the exact native request handler;
      // CEF's Chrome login UI cannot present in an owned offscreen browser.
      // https://github.com/chromiumembedded/cef/issues/3603
      line->AppendSwitch("disable-chrome-login-prompt");
      line->AppendSwitchWithValue("force-webrtc-ip-handling-policy", "disable_non_proxied_udp");
      if (proxy.port) {
        const auto address = proxy.address.find(':') == std::string::npos
          ? proxy.address : "[" + proxy.address + "]";
        line->AppendSwitchWithValue("proxy-server", "http://" + address + ":" + std::to_string(proxy.port));
        line->AppendSwitchWithValue("proxy-bypass-list", "<-loopback>");
        // The numeric proxy connects upstream and resolves reviewed destinations.
        // Direct DNS/UDP denial still belongs to the OS containment boundary.
        line->AppendSwitchWithValue("host-resolver-rules", "MAP * ~NOTFOUND, EXCLUDE " + proxy.address);
      }
    }
  }
  void OnScheduleMessagePumpWork(int64_t delay) override {
    if (callbacks.schedule_pump) callbacks.schedule_pump(callbacks.owner, delay);
  }
  bool OnAlreadyRunningAppRelaunch(CefRefPtr<CefCommandLine>, const CefString&) override {
    // Never let CEF create an unmanaged top-level Chrome window.
    return true;
  }
 private:
  bool no_display_;
#if defined(OS_MAC)
  const std::optional<MacHelperPrograms> owned_helpers_;
#endif
#if defined(COLOSSUS_CEF_TEST_CUSTODY)
  // CEF obtains the handler independently for context creation and incoming
  // process messages. Retain one instance so the challenge sees that context.
  CefRefPtr<CefRenderProcessHandler> custody_renderer_ = CustodyRenderer();
#endif
  IMPLEMENT_REFCOUNTING(App);
};

void Emit(colossus_cef_tab tab, uint64_t generation, uint32_t event,
          int32_t command, int32_t success, const void* data, size_t size) {
  if (!callbacks.event) return;
  if (size > COLOSSUS_CEF_MAX_PROTOCOL_BYTES) {
    event = COLOSSUS_CEF_PROTOCOL_OVERFLOW; data = nullptr; size = 0; success = 0;
  }
  callbacks.event(callbacks.owner, tab, generation, event, command, success,
                  static_cast<const uint8_t*>(data), size);
}

extern "C" int32_t colossus_cef_proxy_configure(const char* address,
  uint16_t port, const char* username, const char* password) {
  using namespace colossus;
  if (initialized || proxy.port) return COLOSSUS_CEF_BUSY;
  if (!address || !username || !password || !port ||
      strnlen(address, 65) > 64 || strnlen(username, 129) > 128 ||
      strnlen(password, 4097) > 4096 || !*username || !*password)
    return COLOSSUS_CEF_INVALID;
  const std::string host(address);
  if (!BootstrapLiteralAddress(host)) return COLOSSUS_CEF_INVALID;
  proxy = {host, port, username, password};
  return COLOSSUS_CEF_OK;
}

extern "C" int32_t colossus_cef_broker_tls_configure(const char* origin,
  const uint8_t* leaf, size_t bytes, uint32_t lifetime_ms) {
#if defined(OS_MAC) && defined(COLOSSUS_CEF_DEVELOPER_PROCESS_BROKER)
  using namespace colossus;
  if (initialized || !proxy.port) return COLOSSUS_CEF_BUSY;
  if (!origin || strnlen(origin, 4097) > 4096 || !*origin)
    return COLOSSUS_CEF_INVALID;
  if (!BootstrapNumericHttpsOrigin(origin)) return COLOSSUS_CEF_INVALID;
  return broker_tls.Configure(origin, leaf, bytes, lifetime_ms,
    std::chrono::steady_clock::now()) ? COLOSSUS_CEF_OK : COLOSSUS_CEF_DENIED;
#else
  (void)origin; (void)leaf; (void)bytes; (void)lifetime_ms;
  return COLOSSUS_CEF_UNAVAILABLE;
#endif
}

bool Allow(colossus_cef_tab tab, uint64_t generation, const std::string& url,
           bool navigation) {
  if (url == "about:blank") return true;
  if (url.size() > 8192) return false;
  CefURLParts parts;
  if (!CefParseURL(url, parts)) return false;
  const auto scheme = CefString(&parts.scheme).ToString();
  if ((scheme != "http" && scheme != "https") || parts.host.length == 0 ||
      parts.username.length != 0 || parts.password.length != 0) return false;
  return callbacks.allow_url && callbacks.allow_url(callbacks.owner, tab, generation,
                                                    url.data(), url.size(), navigation);
}

int32_t Resolve(colossus_cef_tab tab, uint64_t generation, Tab** out) {
  if (!initialized) return COLOSSUS_CEF_UNAVAILABLE;
  if (!CefCurrentlyOn(TID_UI)) return COLOSSUS_CEF_WRONG_THREAD;
  const auto it = tabs.find(tab);
  if (it == tabs.end() || it->second.generation != generation || it->second.closing ||
      !it->second.browser) return COLOSSUS_CEF_CLOSED;
  *out = &it->second;
  return COLOSSUS_CEF_OK;
}

bool ValidBounds(colossus_cef_bounds b) {
  return b.x >= -32768 && b.y >= -32768 && b.x <= 32768 && b.y <= 32768 &&
         b.width > 0 && b.height > 0 && b.width <= 16384 && b.height <= 16384;
}

void State(colossus_cef_tab tab) {
  const auto it = tabs.find(tab);
  if (it == tabs.end() || !it->second.browser) return;
  const auto& t = it->second;
  auto value = CefDictionaryValue::Create();
  auto url = t.browser->GetMainFrame()->GetURL().ToString();
  value->SetString("url", url.substr(0, 8192));
  // Native title callback supplies title separately; never evaluate page JS.
  value->SetBool("canGoBack", t.browser->CanGoBack());
  value->SetBool("canGoForward", t.browser->CanGoForward());
  value->SetBool("loading", t.browser->IsLoading());
  auto wrapper = CefValue::Create(); wrapper->SetDictionary(value);
  auto json = CefWriteJSON(wrapper, JSON_WRITER_DEFAULT).ToString();
  Emit(tab, t.generation, COLOSSUS_CEF_STATE, 0, 1, json.data(), json.size());
}
}

extern "C" int32_t colossus_cef_bootstrap(const colossus_cef_bootstrap_options* o,
                                          int32_t* subprocess_exit) {
  using namespace colossus;
  if (!o || !subprocess_exit || o->abi_version != COLOSSUS_CEF_ABI_VERSION ||
      o->argc < 0 || !o->argv || o->headless < 0 || o->headless > 2) return COLOSSUS_CEF_INVALID;
  *subprocess_exit = -1;
  if (initialized) return COLOSSUS_CEF_BUSY;
  const auto diagnostic = SyntheticProfileBootstrap(o->headless);
  if (diagnostic) { BootstrapDiagnostic("selection", diagnostic); return diagnostic; }
#if defined(OS_MAC)
  if (o->headless == 2 && !PlatformDedicatedEnvironment(o->root_cache_path)) {
    BootstrapDiagnostic("owned-environment", COLOSSUS_CEF_DENIED);
    return COLOSSUS_CEF_DENIED;
  }
  std::optional<MacHelperPrograms> owned_helpers;
  if (o->headless == 2) {
    owned_helpers = PrepareMacHelperPrograms(o->browser_subprocess_path);
    if (!owned_helpers) {
      BootstrapDiagnostic("helper-programs", COLOSSUS_CEF_DENIED);
      return COLOSSUS_CEF_DENIED;
    }
  }
#endif
  if (!PlatformLoadLibrary()) {
    BootstrapDiagnostic("framework", COLOSSUS_CEF_UNAVAILABLE);
    return COLOSSUS_CEF_UNAVAILABLE;
  }
  BootstrapDiagnostic("framework", COLOSSUS_CEF_OK);
#if defined(OS_WIN)
  // New CEF Windows builds require the bootstrap-provided client DLL entry.
  // A raw Tauri executable cannot synthesize this proof or opt out of sandbox.
  uintptr_t instance = 0;
  void* sandbox = nullptr;
  if (colossus_cef_windows_context(&instance, &sandbox) != COLOSSUS_CEF_OK ||
      o->platform_instance != instance || o->sandbox_info != sandbox)
    return COLOSSUS_CEF_UNAVAILABLE;
  CefMainArgs args(reinterpret_cast<HINSTANCE>(o->platform_instance));
#else
  CefMainArgs args(o->argc, o->argv);
#endif
  headless = o->headless != 0;
#if !defined(OS_LINUX)
  if (o->headless == 1) return COLOSSUS_CEF_UNAVAILABLE;
#endif
  callbacks = o->callbacks;
#if defined(OS_LINUX)
  // Both private host placements render offscreen. Embedded presentation travels
  // over the authenticated frame channel and needs no X11/Wayland connection.
  CefRefPtr<App> app = new App(headless);
#elif defined(OS_MAC)
  CefRefPtr<App> app = owned_helpers ? new App(false, std::move(*owned_helpers)) : new App(o->headless == 1);
#else
  CefRefPtr<App> app = new App(o->headless == 1);
#endif
#if !defined(OS_WIN)
  // Windows consumes subprocess entry before Rust/application state exists.
  const int result = CefExecuteProcess(args, app, o->sandbox_info);
  if (result >= 0) { *subprocess_exit = result; return COLOSSUS_CEF_OK; }
#endif
#if defined(OS_MAC)
  if (o->headless == 2) {
    CefString temporary;
    if (!CefGetPath(PK_DIR_TEMP, temporary) ||
        temporary.ToString() != std::string(o->root_cache_path) + "/tmp") {
      BootstrapDiagnostic("owned-temporary-path", COLOSSUS_CEF_DENIED);
      return COLOSSUS_CEF_DENIED;
    }
    BootstrapDiagnostic("owned-temporary-path", COLOSSUS_CEF_OK);
  }
#endif
  if (!o->root_cache_path || !std::filesystem::path(o->root_cache_path).is_absolute() ||
      !o->callbacks.event || !o->callbacks.allow_url) return COLOSSUS_CEF_INVALID;
  if (!ProfileRoot(o->root_cache_path)) {
    BootstrapDiagnostic("profile", COLOSSUS_CEF_DENIED);
    return COLOSSUS_CEF_DENIED;
  }
  BootstrapDiagnostic("profile", COLOSSUS_CEF_OK);
  if (!PlatformEarlySetup()) {
    BootstrapDiagnostic("appkit", COLOSSUS_CEF_UNAVAILABLE);
    return COLOSSUS_CEF_UNAVAILABLE;
  }
  BootstrapDiagnostic("appkit", COLOSSUS_CEF_OK);
  CefSettings settings;
  settings.no_sandbox = false;
  settings.external_message_pump = true;
  settings.windowless_rendering_enabled = headless;
  settings.command_line_args_disabled = true;
  settings.remote_debugging_port = 0;
  settings.log_severity = LOGSEVERITY_DISABLE;
  CefString(&settings.root_cache_path) = o->root_cache_path;
  if (o->browser_subprocess_path) {
    if (!std::filesystem::path(o->browser_subprocess_path).is_absolute())
      return COLOSSUS_CEF_INVALID;
    CefString(&settings.browser_subprocess_path) = o->browser_subprocess_path;
  }
  BootstrapDiagnostic("initialize-enter", COLOSSUS_CEF_OK);
  if (!CefInitialize(args, settings, app, o->sandbox_info)) {
    // CEF's public numeric exit code identifies an early initialization result.
    BootstrapDiagnostic("initialize-exit", COLOSSUS_CEF_UNAVAILABLE, CefGetExitCode());
    return COLOSSUS_CEF_UNAVAILABLE;
  }
  BootstrapDiagnostic("initialize-exit", COLOSSUS_CEF_OK);
  owning_thread = std::this_thread::get_id(); initialized = true;
  return COLOSSUS_CEF_OK;
}

extern "C" int32_t colossus_cef_pump() {
  if (!colossus::initialized) return COLOSSUS_CEF_UNAVAILABLE;
  if (!colossus::OnOwningThread()) return COLOSSUS_CEF_WRONG_THREAD;
  colossus::PlatformEventLoopDiagnostics();
  colossus::PresentationExpire();
  colossus::DownloadExpire();
  CefDoMessageLoopWork(); return COLOSSUS_CEF_OK;
}
extern "C" int32_t colossus_cef_shutdown() {
  using namespace colossus;
  if (!initialized) return COLOSSUS_CEF_UNAVAILABLE;
  if (!CefCurrentlyOn(TID_UI)) return COLOSSUS_CEF_WRONG_THREAD;
  if (!tabs.empty()) return COLOSSUS_CEF_BUSY;
  contexts.clear(); CefShutdown(); initialized = false; callbacks = {};
  ProfileShutdown();
#if defined(COLOSSUS_CEF_TEST_CUSTODY)
  CustodyFinish();
#endif
  std::fill(proxy.username.begin(), proxy.username.end(), '\0');
  std::fill(proxy.password.begin(), proxy.password.end(), '\0');
  proxy = {};
  broker_tls.Revoke();
  return COLOSSUS_CEF_OK;
}
