#include "host_internal.h"
#include "include/cef_dialog_handler.h"
#include "include/cef_download_handler.h"
#include "include/cef_jsdialog_handler.h"
#include "include/cef_permission_handler.h"
#include "include/cef_resource_request_handler.h"
#include "include/cef_render_handler.h"
#include "include/cef_context_menu_handler.h"
#include "include/cef_focus_handler.h"
#include "include/wrapper/cef_helpers.h"
#include <algorithm>
#include "include/cef_task.h"
#include <vector>
#include <limits>
#if defined(COLOSSUS_CEF_TEST_CUSTODY)
#include "custody_linux.h"
#endif

namespace colossus {
void CancelIdentity(colossus_cef_tab tab, Tab& state) {
  if (!state.identity_callback) return;
  const auto request = state.identity_request;
  auto callback = state.identity_callback;
  state.identity_callback = nullptr; state.identity_candidates.clear(); state.identity_request = 0;
  callback->Select(nullptr);
  Emit(tab, state.generation, COLOSSUS_CEF_PKI_SELECTION_CANCELLED, 0, 1,
       &request, sizeof(request));
}
// One sweep task owns expiry for the bounded tab registry. Replacing requests
// cannot accumulate a separate two-minute task for each hostile TLS request.
static bool identity_sweep_pending = false;
static bool ScheduleIdentitySweep();
class IdentityDeadline final : public CefTask {
 public:
  void Execute() override {
    CEF_REQUIRE_UI_THREAD();
    identity_sweep_pending = false;
    if (!initialized) return;
    std::vector<colossus_cef_tab> pending;
    for (const auto& entry : tabs) if (entry.second.identity_callback) pending.push_back(entry.first);
    for (const auto tab : pending) {
      auto it = tabs.find(tab);
      if (it != tabs.end() && it->second.identity_callback &&
          std::chrono::steady_clock::now() >= it->second.identity_deadline)
        CancelIdentity(tab, it->second);
    }
    pending.clear();
    for (const auto& entry : tabs) if (entry.second.identity_callback) pending.push_back(entry.first);
    if (!pending.empty() && !ScheduleIdentitySweep()) {
      for (const auto tab : pending) {
        auto it = tabs.find(tab); if (it != tabs.end()) CancelIdentity(tab, it->second);
      }
    }
  }
 private:
  IMPLEMENT_REFCOUNTING(IdentityDeadline);
};
static bool ScheduleIdentitySweep() {
  if (identity_sweep_pending) return true;
  identity_sweep_pending = CefPostDelayedTask(TID_UI, new IdentityDeadline(), 250);
  return identity_sweep_pending;
}
class ContextBoundary final : public CefRequestContextHandler,
                              public CefResourceRequestHandler {
 public:
  CefRefPtr<CefResourceRequestHandler> GetResourceRequestHandler(
    CefRefPtr<CefBrowser>, CefRefPtr<CefFrame>, CefRefPtr<CefRequest>, bool, bool,
    const CefString&, bool&) override { return this; }
  ReturnValue OnBeforeResourceLoad(CefRefPtr<CefBrowser>, CefRefPtr<CefFrame>,
    CefRefPtr<CefRequest>, CefRefPtr<CefCallback>) override {
    // Requests lacking the tab-owned client (e.g. detached service/shared
    // workers) have no exact tab policy here. Deny until context egress is proved.
    return RV_CANCEL;
  }
 private:
  IMPLEMENT_REFCOUNTING(ContextBoundary);
};
CefRefPtr<CefRequestContextHandler> MakeContextBoundary() { return new ContextBoundary; }
class Client final : public CefClient, public CefLifeSpanHandler,
  public CefLoadHandler, public CefDisplayHandler, public CefRequestHandler,
  public CefResourceRequestHandler, public CefDownloadHandler,
  public CefDialogHandler, public CefJSDialogHandler, public CefPermissionHandler,
  public CefDevToolsMessageObserver, public CefRenderHandler,
  public CefContextMenuHandler, public CefFocusHandler {
 public:
  Client(colossus_cef_tab tab, uint64_t generation, colossus_cef_bounds bounds)
    : tab_(tab), generation_(generation), bounds_(bounds) {}
#if defined(COLOSSUS_CEF_TEST_CUSTODY)
  bool OnProcessMessageReceived(CefRefPtr<CefBrowser> browser, CefRefPtr<CefFrame> frame,
      CefProcessId process, CefRefPtr<CefProcessMessage> message) override {
    return CustodyMessage(tab_, generation_, browser, frame, process, message);
  }
#endif
  CefRefPtr<CefLifeSpanHandler> GetLifeSpanHandler() override { return this; }
  CefRefPtr<CefLoadHandler> GetLoadHandler() override { return this; }
  CefRefPtr<CefDisplayHandler> GetDisplayHandler() override { return this; }
  CefRefPtr<CefRequestHandler> GetRequestHandler() override { return this; }
  CefRefPtr<CefDownloadHandler> GetDownloadHandler() override { return this; }
  CefRefPtr<CefDialogHandler> GetDialogHandler() override { return this; }
  CefRefPtr<CefJSDialogHandler> GetJSDialogHandler() override { return this; }
  CefRefPtr<CefPermissionHandler> GetPermissionHandler() override { return this; }
  CefRefPtr<CefContextMenuHandler> GetContextMenuHandler() override { return this; }
  CefRefPtr<CefFocusHandler> GetFocusHandler() override { return this; }
  void OnBeforeContextMenu(CefRefPtr<CefBrowser>, CefRefPtr<CefFrame>,
    CefRefPtr<CefContextMenuParams>, CefRefPtr<CefMenuModel> model) override { model->Clear(); }
  bool OnSetFocus(CefRefPtr<CefBrowser>, FocusSource source) override {
    if (headless) return !PresentationFocused(tab_, generation_);
    const auto it = tabs.find(tab_);
    return source == FOCUS_SOURCE_NAVIGATION || it == tabs.end() || !it->second.visible;
  }
  void OnProtocolExecution(CefRefPtr<CefBrowser>, CefRefPtr<CefFrame>,
    CefRefPtr<CefRequest>, bool& allow_os_execution) override { allow_os_execution = false; }
  CefRefPtr<CefRenderHandler> GetRenderHandler() override { return headless ? this : nullptr; }
  void GetViewRect(CefRefPtr<CefBrowser>, CefRect& rect) override {
    double scale = 1;
    if (PresentationRect(tab_, generation_, &rect, &scale)) return;
    rect = CefRect(0, 0, bounds_.width, bounds_.height);
  }
  bool GetScreenInfo(CefRefPtr<CefBrowser>, CefScreenInfo& info) override {
    CefRect rect; double scale = 1;
    if (!PresentationRect(tab_, generation_, &rect, &scale)) return false;
    info.device_scale_factor = static_cast<float>(scale);
    info.rect = rect; info.available_rect = rect;
    return true;
  }
  void OnPaint(CefRefPtr<CefBrowser>, PaintElementType kind, const RectList&,
               const void* pixels, int width, int height) override {
    PresentationPaint(tab_, generation_, kind, pixels, width, height);
  }

  void OnAfterCreated(CefRefPtr<CefBrowser> browser) override {
    CEF_REQUIRE_UI_THREAD();
    auto it = tabs.find(tab_);
    if (it == tabs.end() || it->second.generation != generation_) {
      browser->GetHost()->CloseBrowser(true); return;
    }
    it->second.browser = browser;
    PresentationDocument(tab_, generation_);
    if (it->second.closing) { browser->GetHost()->CloseBrowser(true); return; }
    it->second.observer_registration = browser->GetHost()->AddDevToolsMessageObserver(this);
    Emit(tab_, generation_, COLOSSUS_CEF_CREATED); State(tab_);
  }
  bool DoClose(CefRefPtr<CefBrowser> browser) override {
    CEF_REQUIRE_UI_THREAD();
    if (headless) return false;
#if defined(OS_MAC)
    // Every macOS guest is an external Tauri child. Closing one tab must not
    // ask AppKit to close the parent Desktop window or wait for its teardown.
    return PlatformCloseChild(browser->GetHost()->GetWindowHandle());
#else
    return false;
#endif
  }
  void OnBeforeClose(CefRefPtr<CefBrowser>) override {
    CEF_REQUIRE_UI_THREAD();
    auto it = tabs.find(tab_);
    if (it != tabs.end() && it->second.generation == generation_) {
      CancelIdentity(tab_, it->second);
      DownloadClosed(tab_);
      PresentationClosed(tab_);
      const auto context = it->second.context_id;
      it->second.observer_registration = nullptr;
      tabs.erase(it);
      bool used = false;
      for (const auto& entry : tabs) used |= entry.second.context_id == context;
      if (!used) contexts.erase(context);
    }
    Emit(tab_, generation_, COLOSSUS_CEF_CLOSED_EVENT);
  }
  bool OnBeforePopup(CefRefPtr<CefBrowser>, CefRefPtr<CefFrame>, int,
    const CefString& target_url, const CefString&, cef_window_open_disposition_t, bool,
    const CefPopupFeatures&, CefWindowInfo&, CefRefPtr<CefClient>&,
    CefBrowserSettings&, CefRefPtr<CefDictionaryValue>&, bool*) override {
    auto url = target_url.ToString();
    if (Allow(tab_, generation_, url, true))
      Emit(tab_, generation_, COLOSSUS_CEF_POPUP_BLOCKED, 0, 1, url.data(), url.size());
    else Emit(tab_, generation_, COLOSSUS_CEF_BLOCKED);
    return true;
  }
  void OnLoadingStateChange(CefRefPtr<CefBrowser>, bool loading, bool, bool) override {
    Emit(tab_, generation_, COLOSSUS_CEF_LOADING, 0, loading); State(tab_);
  }
  void OnLoadStart(CefRefPtr<CefBrowser>, CefRefPtr<CefFrame> frame,
                   TransitionType) override {
    const auto found = tabs.find(tab_);
    if (frame && frame->IsMain() && found != tabs.end() &&
        found->second.generation == generation_) {
      // OnBeforeBrowse revokes immediately; this separate committed-document
      // epoch also fences snapshots taken while the response was still pending.
      PresentationDocument(tab_, generation_); State(tab_);
#if defined(COLOSSUS_CEF_TEST_CUSTODY)
      CustodyDocument(tab_, generation_, frame);
#endif
    }
  }
  void OnTitleChange(CefRefPtr<CefBrowser>, const CefString& title) override {
    auto value = CefDictionaryValue::Create(); value->SetString("title", title.ToString().substr(0, 4096));
    auto wrapper = CefValue::Create(); wrapper->SetDictionary(value);
    auto json = CefWriteJSON(wrapper, JSON_WRITER_DEFAULT).ToString();
    Emit(tab_, generation_, COLOSSUS_CEF_STATE, 0, 1, json.data(), json.size());
  }
  void OnAddressChange(CefRefPtr<CefBrowser>, CefRefPtr<CefFrame> frame,
                       const CefString&) override { if (frame->IsMain()) State(tab_); }
  void OnLoadError(CefRefPtr<CefBrowser>, CefRefPtr<CefFrame>, ErrorCode code,
                   const CefString&, const CefString&) override {
    if (code != ERR_ABORTED) Emit(tab_, generation_, COLOSSUS_CEF_FAILED);
  }
  bool OnBeforeBrowse(CefRefPtr<CefBrowser>, CefRefPtr<CefFrame> frame,
    CefRefPtr<CefRequest> request, bool, bool) override {
    auto it = tabs.find(tab_);
    if (it != tabs.end() && it->second.generation == generation_)
      CancelIdentity(tab_, it->second);
    if (Allow(tab_, generation_, request->GetURL(), true)) {
      if (frame && frame->IsMain()) PresentationDocument(tab_, generation_);
      return false;
    }
    Emit(tab_, generation_, COLOSSUS_CEF_BLOCKED); return true;
  }
  CefRefPtr<CefResourceRequestHandler> GetResourceRequestHandler(
    CefRefPtr<CefBrowser>, CefRefPtr<CefFrame>, CefRefPtr<CefRequest>, bool, bool,
    const CefString&, bool&) override { return this; }
  ReturnValue OnBeforeResourceLoad(CefRefPtr<CefBrowser>, CefRefPtr<CefFrame>,
    CefRefPtr<CefRequest> request, CefRefPtr<CefCallback>) override {
    if (Allow(tab_, generation_, request->GetURL(), false)) return RV_CONTINUE;
    Emit(tab_, generation_, COLOSSUS_CEF_BLOCKED); return RV_CANCEL;
  }
  void OnResourceRedirect(CefRefPtr<CefBrowser>, CefRefPtr<CefFrame>,
    CefRefPtr<CefRequest>, CefRefPtr<CefResponse>, CefString& new_url) override {
    // This callback runs on IO and cannot cancel synchronously. A fixed local
    // non-network scheme prevents following the denied destination; the atomic
    // intent fence schedules native writer cancellation on the owning UI pump.
    if (Allow(tab_, generation_, new_url.ToString(), false)) return;
    new_url = "about:blank";
    DownloadRedirectDenied(tab_, generation_);
    Emit(tab_, generation_, COLOSSUS_CEF_BLOCKED);
  }
  bool GetAuthCredentials(CefRefPtr<CefBrowser>, const CefString&, bool is_proxy,
    const CefString& host, int port, const CefString&, const CefString&,
    CefRefPtr<CefAuthCallback> callback) override {
    std::string username, password;
    if (!is_proxy || !ProxyCredentials(host.ToString(), port, &username, &password))
      return false;
    callback->Continue(username, password);
    std::fill(username.begin(), username.end(), '\0');
    std::fill(password.begin(), password.end(), '\0');
    return true;
  }
  bool OnCertificateError(CefRefPtr<CefBrowser>, ErrorCode, const CefString&,
    CefRefPtr<CefSSLInfo>, CefRefPtr<CefCallback>) override {
    Emit(tab_, generation_, COLOSSUS_CEF_TLS_FAILED); return false;
  }
  bool OnSelectClientCertificate(CefRefPtr<CefBrowser>, bool proxy,
    const CefString& host, int port, const X509CertificateList& certs,
    CefRefPtr<CefSelectClientCertificateCallback> callback) override {
    CEF_REQUIRE_UI_THREAD();
    auto it = tabs.find(tab_);
    if (it == tabs.end() || it->second.generation != generation_ || it->second.closing ||
        proxy || !callbacks.select_identity || certs.empty() || certs.size() > 64 ||
        port <= 0 || port > 65535) {
      callback->Select(nullptr); Emit(tab_, generation_, COLOSSUS_CEF_PKI_SELECTION_REQUIRED); return true;
    }
    CancelIdentity(tab_, it->second);
    std::vector<std::vector<uint8_t>> bytes;
    std::vector<colossus_cef_certificate> candidates;
    for (const auto& cert : certs) {
      auto der = cert->GetDEREncoded();
      if (!der || der->GetSize() > 65536) { callback->Select(nullptr); return true; }
      bytes.emplace_back(der->GetSize()); der->GetData(bytes.back().data(), bytes.back().size(), 0);
    }
    for (const auto& der : bytes) candidates.push_back({der.data(), der.size()});
    auto hostname = host.ToString();
    if (hostname.find(':') != std::string::npos) hostname = "[" + hostname + "]";
    auto origin = "https://" + hostname + (port == 443 ? "" : ":" + std::to_string(port));
    if (!Allow(tab_, generation_, origin, false)) { callback->Select(nullptr); return true; }
    static uint64_t next_request = 0;
    if (next_request == std::numeric_limits<uint64_t>::max()) { callback->Select(nullptr); return true; }
    const auto request = ++next_request;
    auto index = callbacks.select_identity(callbacks.owner, tab_, generation_, request, origin.data(),
                                           origin.size(), candidates.data(), candidates.size());
    if (index == -2) {
      auto& state = it->second;
      state.identity_request = request; state.identity_callback = callback;
      state.identity_candidates.assign(certs.begin(), certs.end());
      state.identity_deadline = std::chrono::steady_clock::now() + std::chrono::seconds(120);
      if (!ScheduleIdentitySweep()) {
        CancelIdentity(tab_, state); return true;
      }
      Emit(tab_, generation_, COLOSSUS_CEF_PKI_SELECTION_REQUIRED);
      return true;
    }
    const bool selected = index >= 0 && size_t(index) < certs.size();
    callback->Select(selected ? certs[index] : nullptr);
    if (!selected) Emit(tab_, generation_, COLOSSUS_CEF_PKI_SELECTION_REQUIRED);
    return true;
  }
  void OnRenderProcessTerminated(CefRefPtr<CefBrowser>, TerminationStatus,
    int, const CefString&) override {
    auto it = tabs.find(tab_);
    if (it != tabs.end() && it->second.generation == generation_) CancelIdentity(tab_, it->second);
    Emit(tab_, generation_, COLOSSUS_CEF_CRASHED);
  }
  bool CanDownload(CefRefPtr<CefBrowser>, const CefString& url,
                   const CefString& method) override {
    const bool allowed = DownloadCan(tab_, generation_, url.ToString(), method.ToString());
    if (!allowed) Emit(tab_, generation_, COLOSSUS_CEF_DOWNLOAD_BLOCKED);
    return allowed;
  }
  bool OnBeforeDownload(CefRefPtr<CefBrowser>, CefRefPtr<CefDownloadItem> item,
    const CefString&, CefRefPtr<CefBeforeDownloadCallback> callback) override {
    return DownloadBefore(tab_, generation_, item, callback);
  }
  void OnDownloadUpdated(CefRefPtr<CefBrowser>, CefRefPtr<CefDownloadItem> item,
    CefRefPtr<CefDownloadItemCallback> callback) override {
    DownloadUpdated(tab_, generation_, item, callback);
  }
  bool OnFileDialog(CefRefPtr<CefBrowser>, FileDialogMode, const CefString&,
    const CefString&, const std::vector<CefString>&, const std::vector<CefString>&,
    const std::vector<CefString>&, CefRefPtr<CefFileDialogCallback> callback) override {
    callback->Cancel(); return true;
  }
  bool OnJSDialog(CefRefPtr<CefBrowser>, const CefString&, JSDialogType,
    const CefString&, const CefString&, CefRefPtr<CefJSDialogCallback>, bool& suppress) override {
    suppress = true; return false;
  }
  bool OnBeforeUnloadDialog(CefRefPtr<CefBrowser>, const CefString&, bool,
    CefRefPtr<CefJSDialogCallback> callback) override { callback->Continue(true, ""); return true; }
  bool OnRequestMediaAccessPermission(CefRefPtr<CefBrowser>, CefRefPtr<CefFrame>,
    const CefString&, uint32_t, CefRefPtr<CefMediaAccessCallback> callback) override {
    callback->Cancel(); return true;
  }
  bool OnShowPermissionPrompt(CefRefPtr<CefBrowser>, uint64_t, const CefString&,
    uint32_t, CefRefPtr<CefPermissionPromptCallback> callback) override {
    callback->Continue(CEF_PERMISSION_RESULT_DENY); return true;
  }
  void OnDevToolsMethodResult(CefRefPtr<CefBrowser>, int id, bool success,
    const void* result, size_t size) override {
    auto it = tabs.find(tab_);
    if (id == std::numeric_limits<int32_t>::max() && it != tabs.end() &&
        it->second.generation == generation_ && it->second.acceptance_probe_pending) {
      it->second.acceptance_probe_pending = false;
      std::string evidence;
      const bool accepted = success && PlatformAcceptanceEvidence(
        it->second.browser->GetHost()->GetWindowHandle(), result, size, &evidence);
      Emit(tab_, generation_, COLOSSUS_CEF_ACCEPTANCE_EVIDENCE, id, accepted,
           evidence.data(), evidence.size());
      return;
    }
    Emit(tab_, generation_, COLOSSUS_CEF_DEVTOOLS_RESULT, id, success, result, size);
  }
  void OnDevToolsEvent(CefRefPtr<CefBrowser>, const CefString& method,
    const void* params, size_t size) override {
    if (size > COLOSSUS_CEF_MAX_PROTOCOL_BYTES) { Emit(tab_, generation_, COLOSSUS_CEF_PROTOCOL_OVERFLOW); return; }
    auto value = CefDictionaryValue::Create(); value->SetString("method", method);
    if (params && size) {
      auto parsed = CefParseJSON(params, size, JSON_PARSER_RFC);
      if (parsed) value->SetValue("params", parsed);
    }
    auto wrapper = CefValue::Create(); wrapper->SetDictionary(value);
    auto json = CefWriteJSON(wrapper, JSON_WRITER_DEFAULT).ToString();
    Emit(tab_, generation_, COLOSSUS_CEF_DEVTOOLS_EVENT, 0, 1, json.data(), json.size());
  }
  void OnDevToolsAgentDetached(CefRefPtr<CefBrowser>) override {
    Emit(tab_, generation_, COLOSSUS_CEF_DEVTOOLS_DETACHED);
  }
 private:
  colossus_cef_tab tab_;
  uint64_t generation_;
  colossus_cef_bounds bounds_;
  IMPLEMENT_REFCOUNTING(Client);
};
CefRefPtr<CefClient> MakeClient(colossus_cef_tab tab, uint64_t generation, colossus_cef_bounds bounds) {
  return new Client(tab, generation, bounds);
}
}
