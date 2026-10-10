#include "host_internal.h"
#include "include/cef_app.h"
#include "include/cef_command_line.h"
#include "include/wrapper/cef_helpers.h"
#include <cstring>
#include <filesystem>
#include <thread>

namespace colossus {
colossus_cef_callbacks callbacks{};
std::map<colossus_cef_tab, Tab> tabs;
std::map<uint64_t, CefRefPtr<CefRequestContext>> contexts;
bool initialized = false;
bool headless = false;
static std::thread::id owning_thread;

class App final : public CefApp, public CefBrowserProcessHandler {
 public:
  explicit App(bool no_display) : no_display_(no_display) {}
  CefRefPtr<CefBrowserProcessHandler> GetBrowserProcessHandler() override { return this; }
  void OnBeforeCommandLineProcessing(const CefString& process,
                                    CefRefPtr<CefCommandLine> line) override {
    if (process.empty()) {
      if (no_display_) {
        line->AppendSwitchWithValue("ozone-platform", "headless");
        line->AppendSwitch("disable-gpu");
      }
      line->AppendSwitch("disable-quic");
      line->AppendSwitchWithValue("force-webrtc-ip-handling-policy", "disable_non_proxied_udp");
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
      o->argc < 0 || !o->argv) return COLOSSUS_CEF_INVALID;
  *subprocess_exit = -1;
  if (initialized) return COLOSSUS_CEF_BUSY;
  if (!PlatformLoadLibrary()) return COLOSSUS_CEF_UNAVAILABLE;
#if defined(OS_WIN)
  // New CEF Windows builds require the bootstrap-provided client DLL entry.
  // A raw Tauri executable cannot synthesize this proof or opt out of sandbox.
  if (!o->platform_instance || !o->sandbox_info) return COLOSSUS_CEF_UNAVAILABLE;
  CefMainArgs args(reinterpret_cast<HINSTANCE>(o->platform_instance));
#else
  CefMainArgs args(o->argc, o->argv);
#endif
  headless = o->headless != 0;
#if !defined(OS_LINUX)
  if (headless) return COLOSSUS_CEF_UNAVAILABLE;
#endif
  callbacks = o->callbacks;
  CefRefPtr<App> app = new App(headless);
  const int result = CefExecuteProcess(args, app, o->sandbox_info);
  if (result >= 0) { *subprocess_exit = result; return COLOSSUS_CEF_OK; }
  if (!o->root_cache_path || !std::filesystem::path(o->root_cache_path).is_absolute() ||
      !o->callbacks.event || !o->callbacks.allow_url) return COLOSSUS_CEF_INVALID;
  if (!PlatformEarlySetup()) return COLOSSUS_CEF_UNAVAILABLE;
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
  if (!CefInitialize(args, settings, app, o->sandbox_info)) return COLOSSUS_CEF_UNAVAILABLE;
  owning_thread = std::this_thread::get_id(); initialized = true;
  return COLOSSUS_CEF_OK;
}

extern "C" int32_t colossus_cef_pump() {
  if (!colossus::initialized) return COLOSSUS_CEF_UNAVAILABLE;
  if (std::this_thread::get_id() != colossus::owning_thread) return COLOSSUS_CEF_WRONG_THREAD;
  colossus::PlatformEventLoopDiagnostics();
  CefDoMessageLoopWork(); return COLOSSUS_CEF_OK;
}
extern "C" int32_t colossus_cef_shutdown() {
  using namespace colossus;
  if (!initialized) return COLOSSUS_CEF_UNAVAILABLE;
  if (!CefCurrentlyOn(TID_UI)) return COLOSSUS_CEF_WRONG_THREAD;
  if (!tabs.empty()) return COLOSSUS_CEF_BUSY;
  contexts.clear(); CefShutdown(); initialized = false; callbacks = {};
  return COLOSSUS_CEF_OK;
}
