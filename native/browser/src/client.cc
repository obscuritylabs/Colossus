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
#include <vector>
#include <limits>

namespace colossus {
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
    const auto it = tabs.find(tab_);
    return source == FOCUS_SOURCE_NAVIGATION || it == tabs.end() || !it->second.visible;
  }
  void OnProtocolExecution(CefRefPtr<CefBrowser>, CefRefPtr<CefFrame>,
    CefRefPtr<CefRequest>, bool& allow_os_execution) override { allow_os_execution = false; }
  CefRefPtr<CefRenderHandler> GetRenderHandler() override { return headless ? this : nullptr; }
  void GetViewRect(CefRefPtr<CefBrowser>, CefRect& rect) override {
    rect = CefRect(0, 0, bounds_.width, bounds_.height);
  }
  void OnPaint(CefRefPtr<CefBrowser>, PaintElementType, const RectList&,
               const void*, int, int) override {}

  void OnAfterCreated(CefRefPtr<CefBrowser> browser) override {
    CEF_REQUIRE_UI_THREAD();
    auto it = tabs.find(tab_);
    if (it == tabs.end() || it->second.generation != generation_) {
      browser->GetHost()->CloseBrowser(true); return;
    }
    it->second.browser = browser;
    if (it->second.closing) { browser->GetHost()->CloseBrowser(true); return; }
    it->second.observer_registration = browser->GetHost()->AddDevToolsMessageObserver(this);
    Emit(tab_, generation_, COLOSSUS_CEF_CREATED); State(tab_);
  }
  bool DoClose(CefRefPtr<CefBrowser> browser) override {
    CEF_REQUIRE_UI_THREAD();
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
  bool OnBeforeBrowse(CefRefPtr<CefBrowser>, CefRefPtr<CefFrame>,
    CefRefPtr<CefRequest> request, bool, bool) override {
    if (Allow(tab_, generation_, request->GetURL(), true)) return false;
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
  bool OnCertificateError(CefRefPtr<CefBrowser>, ErrorCode, const CefString&,
    CefRefPtr<CefSSLInfo>, CefRefPtr<CefCallback>) override {
    Emit(tab_, generation_, COLOSSUS_CEF_TLS_FAILED); return false;
  }
  bool OnSelectClientCertificate(CefRefPtr<CefBrowser>, bool proxy,
    const CefString& host, int port, const X509CertificateList& certs,
    CefRefPtr<CefSelectClientCertificateCallback> callback) override {
    if (proxy || !callbacks.select_identity || certs.size() > 64) {
      callback->Select(nullptr); Emit(tab_, generation_, COLOSSUS_CEF_PKI_SELECTION_REQUIRED); return true;
    }
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
    auto index = callbacks.select_identity(callbacks.owner, tab_, generation_, origin.data(),
                                           origin.size(), candidates.data(), candidates.size());
    const bool selected = index >= 0 && size_t(index) < certs.size();
    callback->Select(selected ? certs[index] : nullptr);
    if (!selected) Emit(tab_, generation_, COLOSSUS_CEF_PKI_SELECTION_REQUIRED);
    return true;
  }
  void OnRenderProcessTerminated(CefRefPtr<CefBrowser>, TerminationStatus,
    int, const CefString&) override { Emit(tab_, generation_, COLOSSUS_CEF_CRASHED); }
  bool OnBeforeDownload(CefRefPtr<CefBrowser>, CefRefPtr<CefDownloadItem>,
    const CefString&, CefRefPtr<CefBeforeDownloadCallback>) override {
    Emit(tab_, generation_, COLOSSUS_CEF_DOWNLOAD_BLOCKED); return true;
  }
  void OnDownloadUpdated(CefRefPtr<CefBrowser>, CefRefPtr<CefDownloadItem>,
    CefRefPtr<CefDownloadItemCallback> callback) override { callback->Cancel(); }
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
