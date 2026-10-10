#include "host_internal.h"
#include "colossus_cef_presentation.h"
#include "colossus_cef_transfer.h"
#include "include/cef_download_handler.h"
#include <algorithm>
#include <atomic>
#include <cstring>
#include <memory>
#include <mutex>

namespace colossus {
namespace {
struct Download {
  uint64_t generation = 0, document = 0, max_bytes = 0;
  uint32_t item_id = 0, status = COLOSSUS_CEF_DOWNLOAD_ARMED;
  uint64_t received = 0, total = 0;
  bool bound = false, started = false, continued = false, cancel_requested = false;
  std::string nonce, path, original_url, final_url;
  CefRefPtr<CefDownloadItemCallback> callback;
  std::shared_ptr<std::atomic<bool>> redirect_blocked;
  std::chrono::steady_clock::time_point deadline;
};
std::map<colossus_cef_tab, Download> downloads;
// Redirect callbacks run on the IO thread. They never read the UI-owned ledger
// or call a UI-owned download callback. At most one atomic fence per owned tab
// is published while arming; its owner clears it only after native tab close.
std::mutex redirect_mutex;
std::map<std::pair<colossus_cef_tab, uint64_t>, std::shared_ptr<std::atomic<bool>>> redirect_fences;
bool Nonce(const char* value) {
  if (!value || strnlen(value, 33) != 32) return false;
  return std::all_of(value, value + 32, [](char character) {
    return (character >= '0' && character <= '9') ||
      (character >= 'a' && character <= 'f');
  });
}
bool Text(const char* value, size_t limit) {
  if (!value || !*value || strnlen(value, limit + 1) > limit) return false;
  for (const unsigned char* byte = reinterpret_cast<const unsigned char*>(value); *byte; ++byte)
    if (*byte < 32 || *byte == 127) return false;
  return true;
}
bool AbsolutePath(const char* path) {
#if defined(OS_WIN)
  return path && std::strlen(path) >= 3 &&
    ((path[0] >= 'A' && path[0] <= 'Z') || (path[0] >= 'a' && path[0] <= 'z')) &&
    path[1] == ':' && (path[2] == '\\' || path[2] == '/');
#else
  return path && path[0] == '/';
#endif
}
bool NetworkUrl(const std::string& url) {
  if (url.empty() || url.size() > COLOSSUS_CEF_MAX_TRANSFER_URL) return false;
  CefURLParts parts;
  if (!CefParseURL(url, parts)) return false;
  const auto scheme = CefString(&parts.scheme).ToString();
  return (scheme == "http" || scheme == "https") && parts.host.length != 0 &&
    parts.username.length == 0 && parts.password.length == 0;
}
bool Terminal(const Download& state) { return state.status >= COLOSSUS_CEF_DOWNLOAD_COMPLETE; }
bool Current(colossus_cef_tab tab, const Download& state) {
  uint64_t document = 0;
  return colossus_cef_presentation_document(tab, state.generation, &document) == COLOSSUS_CEF_OK &&
    document == state.document;
}
void Cancel(Download& state) {
  state.cancel_requested = true;
  state.final_url.clear();
  if (!state.continued) state.status = COLOSSUS_CEF_DOWNLOAD_CANCELLED;
  if (state.callback && !Terminal(state)) state.callback->Cancel();
}
bool Live(colossus_cef_tab tab, Download& state) {
  if (state.cancel_requested || state.redirect_blocked->load(std::memory_order_acquire) || !Current(tab, state) ||
      std::chrono::steady_clock::now() >= state.deadline) {
    if (!Terminal(state)) Cancel(state);
    return false;
  }
  return true;
}
bool Item(colossus_cef_tab tab, Download& state, CefRefPtr<CefDownloadItem> item) {
  if (!item || !item->IsValid() || !Live(tab, state) || Terminal(state)) return false;
  const auto original = item->GetOriginalUrl().ToString();
  const auto url = item->GetURL().ToString();
  const auto path = item->GetFullPath().ToString();
  const int64_t received = item->GetReceivedBytes(), total = item->GetTotalBytes();
  return (!state.bound || state.item_id == item->GetId()) && original == state.original_url &&
    NetworkUrl(url) &&
    Allow(tab, state.generation, url, false) &&
    (path.empty() || path == state.path || (!item->IsComplete() && path == state.path + ".crdownload")) &&
    (!item->IsComplete() || path == state.path) && received >= 0 &&
    uint64_t(received) <= state.max_bytes && (total < 0 || uint64_t(total) <= state.max_bytes);
}
int32_t Lookup(colossus_cef_tab tab, uint64_t generation, uint64_t document,
               const char* nonce, Download** state) {
  Tab* browser = nullptr;
  const auto status = Resolve(tab, generation, &browser);
  if (status) return status;
  if (!Nonce(nonce) || !document) return COLOSSUS_CEF_INVALID;
  const auto found = downloads.find(tab);
  if (found == downloads.end() || found->second.generation != generation ||
      found->second.document != document || found->second.nonce != nonce) return COLOSSUS_CEF_DENIED;
  *state = &found->second;
  if (!Current(tab, **state)) { if (!Terminal(**state)) Cancel(**state); return COLOSSUS_CEF_DENIED; }
  return COLOSSUS_CEF_OK;
}
}

bool DownloadCan(colossus_cef_tab tab, uint64_t generation, const std::string& url,
                 const std::string& method) {
  // StartDownload bypasses CanDownload. Every page-initiated request is denied,
  // so an unrelated page click cannot consume even an exact-URL armed intent.
  return false;
}
bool DownloadBefore(colossus_cef_tab tab, uint64_t generation,
                    CefRefPtr<CefDownloadItem> item, CefRefPtr<CefBeforeDownloadCallback> callback) {
  const auto found = downloads.find(tab);
  if (found == downloads.end() || found->second.generation != generation ||
      !found->second.started || found->second.continued || !callback || !Item(tab, found->second, item)) {
    Emit(tab, generation, COLOSSUS_CEF_DOWNLOAD_BLOCKED); return true;
  }
  auto& state = found->second;
  state.bound = true; state.item_id = item->GetId(); state.continued = true;
  state.status = COLOSSUS_CEF_DOWNLOAD_WRITING;
  // The file is always generated by the trusted host; page filenames and the
  // platform's default download directory/save dialog are never consulted.
  callback->Continue(state.path, false);
  return true;
}
void DownloadUpdated(colossus_cef_tab tab, uint64_t generation,
                     CefRefPtr<CefDownloadItem> item, CefRefPtr<CefDownloadItemCallback> callback) {
  const auto found = downloads.find(tab);
  if (!item || !item->IsValid() || found == downloads.end() ||
      found->second.generation != generation ||
      (found->second.bound && found->second.item_id != item->GetId())) {
    if (callback) callback->Cancel();
    return;
  }
  auto& state = found->second;
  if (Terminal(state)) {
    if (state.status != COLOSSUS_CEF_DOWNLOAD_COMPLETE && callback) callback->Cancel();
    return;
  }
  const bool accepted = state.started && Item(tab, state, item);
  if (!accepted) {
    if (callback) callback->Cancel();
    // A page-originated download with another original URL cannot consume or
    // overwrite the armed command's intent or quiescence state.
    if (!state.bound) return;
    state.cancel_requested = true; state.final_url.clear();
  } else {
    state.bound = true; state.item_id = item->GetId();
    state.callback = callback;
    state.received = uint64_t(item->GetReceivedBytes());
    const int64_t total = item->GetTotalBytes(); state.total = total < 0 ? 0 : uint64_t(total);
    state.final_url = item->GetURL().ToString();
  }
  if (item->IsCanceled()) state.status = COLOSSUS_CEF_DOWNLOAD_CANCELLED;
  else if (item->IsInterrupted()) state.status = COLOSSUS_CEF_DOWNLOAD_FAILED;
  else if (item->IsComplete()) {
    state.status = accepted && state.continued && !state.cancel_requested
      ? COLOSSUS_CEF_DOWNLOAD_COMPLETE : COLOSSUS_CEF_DOWNLOAD_FAILED;
    if (state.status == COLOSSUS_CEF_DOWNLOAD_COMPLETE) state.total = state.received;
  }
  if (Terminal(state)) {
    state.callback = nullptr;
    if (state.status != COLOSSUS_CEF_DOWNLOAD_COMPLETE) state.final_url.clear();
  }
}
void DownloadExpire() {
  for (auto& [tab, state] : downloads) if (!Terminal(state)) Live(tab, state);
}
void DownloadRedirectDenied(colossus_cef_tab tab, uint64_t generation) {
  std::lock_guard<std::mutex> guard(redirect_mutex);
  const auto found = redirect_fences.find({tab, generation});
  if (found != redirect_fences.end()) found->second->store(true, std::memory_order_release);
}
void DownloadClosed(colossus_cef_tab tab) {
  const auto found = downloads.find(tab);
  if (found == downloads.end()) return;
  if (!Terminal(found->second)) Cancel(found->second);
  {
    std::lock_guard<std::mutex> guard(redirect_mutex);
    redirect_fences.erase({tab, found->second.generation});
  }
  downloads.erase(found);
}
}

extern "C" int32_t colossus_cef_download_arm(colossus_cef_tab tab, uint64_t generation,
    uint64_t document, const char* nonce, const char* path, const char* url,
    uint64_t max_bytes, uint32_t ttl_ms) {
  using namespace colossus;
  Tab* browser = nullptr;
  const auto status = Resolve(tab, generation, &browser);
  if (status) return status;
  if (!document || !Nonce(nonce) || !Text(path, 4096) || !AbsolutePath(path) ||
      !Text(url, COLOSSUS_CEF_MAX_TRANSFER_URL) || !max_bytes ||
      max_bytes > COLOSSUS_CEF_MAX_DOWNLOAD_BYTES || !ttl_ms || ttl_ms > 30000)
    return COLOSSUS_CEF_INVALID;
  uint64_t actual_document = 0;
  if (colossus_cef_presentation_document(tab, generation, &actual_document) != COLOSSUS_CEF_OK ||
      document != actual_document || !NetworkUrl(url) || !Allow(tab, generation, url, false)) return COLOSSUS_CEF_DENIED;
  const auto previous = downloads.find(tab);
  if (previous != downloads.end() &&
      (!Terminal(previous->second) || previous->second.nonce == nonce)) return COLOSSUS_CEF_BUSY;
  Download state;
  state.generation = generation; state.document = document; state.nonce = nonce;
  state.path = path; state.original_url = url; state.max_bytes = max_bytes;
  state.deadline = std::chrono::steady_clock::now() + std::chrono::milliseconds(ttl_ms);
  state.redirect_blocked = std::make_shared<std::atomic<bool>>(false);
  {
    std::lock_guard<std::mutex> guard(redirect_mutex);
    redirect_fences.insert_or_assign({tab, generation}, state.redirect_blocked);
  }
  downloads.insert_or_assign(tab, std::move(state));
  return COLOSSUS_CEF_OK;
}
extern "C" int32_t colossus_cef_download_start(colossus_cef_tab tab, uint64_t generation,
    uint64_t document, const char* nonce) {
  using namespace colossus;
  Download* state = nullptr;
  const auto status = Lookup(tab, generation, document, nonce, &state);
  if (status) return status;
  if (!Live(tab, *state) || Terminal(*state) || state->started ||
      !Allow(tab, generation, state->original_url, false)) return COLOSSUS_CEF_DENIED;
  Tab* browser = nullptr;
  const auto native_status = Resolve(tab, generation, &browser);
  if (native_status) return native_status;
  state->started = true;
  browser->browser->GetHost()->StartDownload(state->original_url);
  return COLOSSUS_CEF_OK;
}
extern "C" int32_t colossus_cef_download_poll(colossus_cef_tab tab, uint64_t generation,
    uint64_t document, const char* nonce, colossus_cef_download_state* result) {
  using namespace colossus;
  if (!result) return COLOSSUS_CEF_INVALID;
  Download* state = nullptr;
  const auto status = Lookup(tab, generation, document, nonce, &state);
  if (status) return status;
  if (!Terminal(*state)) Live(tab, *state);
  *result = {};
  result->status = state->status; result->received_bytes = state->received;
  result->total_bytes = state->total;
  if (!state->final_url.empty()) {
    result->final_url_len = static_cast<uint32_t>(state->final_url.size());
    std::memcpy(result->final_url, state->final_url.data(), state->final_url.size());
  }
  return COLOSSUS_CEF_OK;
}
extern "C" int32_t colossus_cef_download_cancel(colossus_cef_tab tab, uint64_t generation,
    uint64_t document, const char* nonce) {
  using namespace colossus;
  Download* state = nullptr;
  const auto status = Lookup(tab, generation, document, nonce, &state);
  if (status) return status;
  if (!Terminal(*state)) Cancel(*state);
  return COLOSSUS_CEF_OK;
}
