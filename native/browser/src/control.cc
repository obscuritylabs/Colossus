#include "host_internal.h"
#include "include/wrapper/cef_helpers.h"
#include <cstring>
#include <limits>

extern "C" int32_t colossus_cef_create(colossus_cef_tab tab, uint64_t generation,
  uint64_t context_id, uintptr_t parent, colossus_cef_bounds bounds, const char* url) {
  using namespace colossus;
  if (!initialized) return COLOSSUS_CEF_UNAVAILABLE;
  if (!CefCurrentlyOn(TID_UI)) return COLOSSUS_CEF_WRONG_THREAD;
  if (!tab || !generation || !context_id || !url || strnlen(url, 8193) > 8192 ||
      !ValidBounds(bounds) || tabs.size() >= 8)
    return COLOSSUS_CEF_INVALID;
  if (tabs.count(tab)) return COLOSSUS_CEF_BUSY;
  if (!Allow(tab, generation, url, true)) return COLOSSUS_CEF_DENIED;
  if (!headless && !parent) return COLOSSUS_CEF_INVALID;
  if (headless && parent) return COLOSSUS_CEF_INVALID;
#if defined(OS_WIN)
  DWORD parent_process = 0;
  if (!GetWindowThreadProcessId(reinterpret_cast<HWND>(parent), &parent_process) ||
      parent_process != GetCurrentProcessId()) return COLOSSUS_CEF_DENIED;
#endif
  auto& context = contexts[context_id];
  if (!context) { CefRequestContextSettings settings;
    context = CefRequestContext::CreateContext(settings, MakeContextBoundary()); }
  auto client = MakeClient(tab, generation, bounds);
  tabs.emplace(tab, Tab{generation, context_id, nullptr, client, nullptr, false});
  CefWindowInfo window;
  auto child_bounds = PlatformChildBounds(parent, bounds);
#if defined(OS_LINUX)
  window.SetAsChild(static_cast<CefWindowHandle>(parent), CefRect(child_bounds.x, child_bounds.y, child_bounds.width, child_bounds.height));
#else
  window.SetAsChild(reinterpret_cast<CefWindowHandle>(parent), CefRect(child_bounds.x, child_bounds.y, child_bounds.width, child_bounds.height));
#endif
#if defined(OS_WIN)
  window.style &= ~WS_VISIBLE;
#elif defined(OS_MAC)
  window.hidden = true;
#endif
  if (headless) window.SetAsWindowless(0);
  window.runtime_style = CEF_RUNTIME_STYLE_ALLOY;
  CefBrowserSettings settings;
  settings.webgl = STATE_DISABLED;
  if (!CefBrowserHost::CreateBrowser(window, client, url, settings, nullptr, context)) {
    tabs.erase(tab); bool used = false;
    for (const auto& entry : tabs) used |= entry.second.context_id == context_id;
    if (!used) contexts.erase(context_id);
    return COLOSSUS_CEF_UNAVAILABLE;
  }
  return COLOSSUS_CEF_OK;
}

extern "C" int32_t colossus_cef_navigate(colossus_cef_tab tab, uint64_t generation, const char* url) {
  colossus::Tab* t; auto status = colossus::Resolve(tab, generation, &t);
  if (status) return status;
  if (!url || strnlen(url, 8193) > 8192) return COLOSSUS_CEF_INVALID;
  if (!colossus::Allow(tab, generation, url, true)) return COLOSSUS_CEF_DENIED;
  t->browser->GetMainFrame()->LoadURL(url); return COLOSSUS_CEF_OK;
}
extern "C" int32_t colossus_cef_control(colossus_cef_tab tab, uint64_t generation, uint32_t action) {
  colossus::Tab* t; auto status = colossus::Resolve(tab, generation, &t);
  if (status) return status;
  switch (action) {
    case COLOSSUS_CEF_BACK: t->browser->GoBack(); break;
    case COLOSSUS_CEF_FORWARD: t->browser->GoForward(); break;
    case COLOSSUS_CEF_RELOAD: t->browser->Reload(); break;
    case COLOSSUS_CEF_STOP: t->browser->StopLoad(); break;
    default: return COLOSSUS_CEF_INVALID;
  }
  return COLOSSUS_CEF_OK;
}
extern "C" int32_t colossus_cef_inspect(colossus_cef_tab tab, uint64_t generation) {
  colossus::Tab* t; auto status = colossus::Resolve(tab, generation, &t);
  if (!status) colossus::State(tab);
  return status;
}
extern "C" int32_t colossus_cef_bounds_set(colossus_cef_tab tab, uint64_t generation, colossus_cef_bounds bounds) {
  colossus::Tab* t; auto status = colossus::Resolve(tab, generation, &t);
  if (status) return status;
  if (!colossus::ValidBounds(bounds)) return COLOSSUS_CEF_INVALID;
  if (colossus::headless) return COLOSSUS_CEF_UNAVAILABLE;
  return colossus::PlatformBounds(t->browser->GetHost()->GetWindowHandle(), bounds);
}
extern "C" int32_t colossus_cef_visible(colossus_cef_tab tab, uint64_t generation, int32_t visible) {
  colossus::Tab* t; auto status = colossus::Resolve(tab, generation, &t);
  if (status) return status;
  if (colossus::headless) return COLOSSUS_CEF_UNAVAILABLE;
  if (!visible) t->browser->GetHost()->SetFocus(false);
  status = colossus::PlatformVisible(t->browser->GetHost()->GetWindowHandle(), visible != 0);
  t->visible = !status && visible;
  return status;
}
extern "C" int32_t colossus_cef_focus(colossus_cef_tab tab, uint64_t generation) {
  colossus::Tab* t; auto status = colossus::Resolve(tab, generation, &t);
  if (!status && !t->visible) return COLOSSUS_CEF_DENIED;
  if (!status) t->browser->GetHost()->SetFocus(true);
  return status;
}
extern "C" int32_t colossus_cef_close(colossus_cef_tab tab, uint64_t generation) {
  using namespace colossus;
  if (!initialized) return COLOSSUS_CEF_UNAVAILABLE;
  if (!CefCurrentlyOn(TID_UI)) return COLOSSUS_CEF_WRONG_THREAD;
  auto it = tabs.find(tab);
  if (it == tabs.end() || it->second.generation != generation) return COLOSSUS_CEF_CLOSED;
  it->second.closing = true;
  if (it->second.browser) it->second.browser->GetHost()->CloseBrowser(true);
  // Pending CreateBrowser is cancelled in OnAfterCreated and acknowledged closed.
  return COLOSSUS_CEF_OK;
}
extern "C" int32_t colossus_cef_devtools(colossus_cef_tab tab, uint64_t generation,
  int32_t command, const char* method, const uint8_t* params, size_t size) {
  colossus::Tab* t; auto status = colossus::Resolve(tab, generation, &t);
  if (status) return status;
  if (command <= 0 || command == std::numeric_limits<int32_t>::max() ||
      !method || strnlen(method, 257) > 256 ||
      !params || !size || size > COLOSSUS_CEF_MAX_PROTOCOL_BYTES) return COLOSSUS_CEF_INVALID;
  auto parsed = CefParseJSON(params, size, JSON_PARSER_RFC);
  if (!parsed || parsed->GetType() != VTYPE_DICTIONARY) return COLOSSUS_CEF_INVALID;
  if (!t->browser->GetHost()->ExecuteDevToolsMethod(command, method, parsed->GetDictionary()))
    return COLOSSUS_CEF_UNAVAILABLE;
  return COLOSSUS_CEF_OK;
}

extern "C" int32_t colossus_cef_acceptance_probe(colossus_cef_tab tab, uint64_t generation) {
  colossus::Tab* t; auto status = colossus::Resolve(tab, generation, &t);
  if (status) return status;
#if !defined(OS_MAC)
  return COLOSSUS_CEF_UNAVAILABLE;
#else
  if (t->acceptance_probe_pending) return COLOSSUS_CEF_BUSY;
  auto params = CefDictionaryValue::Create();
  params->SetString("format", "png");
  params->SetBool("captureBeyondViewport", false);
  t->acceptance_probe_pending = true;
  if (!t->browser->GetHost()->ExecuteDevToolsMethod(std::numeric_limits<int32_t>::max(),
                                                    "Page.captureScreenshot", params)) {
    t->acceptance_probe_pending = false;
    return COLOSSUS_CEF_UNAVAILABLE;
  }
  return COLOSSUS_CEF_OK;
#endif
}

extern "C" int32_t colossus_cef_acceptance_activate(colossus_cef_tab tab, uint64_t generation) {
  colossus::Tab* t; auto status = colossus::Resolve(tab, generation, &t);
  if (status) return status;
#if defined(OS_MAC)
  return colossus::PlatformAcceptanceActivate(t->browser->GetHost()->GetWindowHandle());
#else
  return COLOSSUS_CEF_UNAVAILABLE;
#endif
}

extern "C" int32_t colossus_cef_acceptance_terminate(colossus_cef_tab tab, uint64_t generation) {
  colossus::Tab* t; auto status = colossus::Resolve(tab, generation, &t);
  if (status) return status;
#if defined(OS_MAC)
  return colossus::PlatformAcceptanceTerminate(t->browser->GetHost()->GetWindowHandle());
#else
  return COLOSSUS_CEF_UNAVAILABLE;
#endif
}
