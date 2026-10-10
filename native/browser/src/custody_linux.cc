// Closed experiment: a native renderer attempts to OPEN (never read) the owned
// NSS key DB. It establishes only direct-syscall denial, not broker key custody.
#include "host_internal.h"
#include "custody_linux.h"
#include "custody_proc_linux.h"
#include "include/cef_v8.h"
#include "include/wrapper/cef_helpers.h"
#include <array>
#include <cerrno>
#include <cstdlib>
#include <cstring>
#include <cstdio>
#include <fcntl.h>
#include <limits.h>
#include <limits>
#include <set>
#include <sys/random.h>
#include <sys/stat.h>
#include <unistd.h>

namespace colossus {
namespace {
constexpr char kName[] = "ColossusNativeOwnedNssOpenExperimentV1";
constexpr char kReady[] = "ColossusNativeOwnedNssOpenReadyV1";
constexpr char kChallenge[] = "ColossusNativeOwnedNssOpenChallengeV1";
constexpr char kNonceSwitch[] = "colossus-native-nss-open-test";
constexpr char kUrlSwitch[] = "colossus-native-nss-open-fixture";
struct Prepared {
  int descriptor = -1;
  struct stat identity{};
  std::string path, url, launch_nonce;
  std::set<std::string> launches;
  custody_test::BrowserIdentity browser;
  colossus_cef_tab tab = 0;
  uint64_t generation = 0, document = 0, pending_document = 0;
  std::string frame, challenge;
  pid_t renderer_pid = 0;
  std::chrono::steady_clock::time_point deadline;
  bool consumed = false;
};
Prepared prepared;
bool Nonce(std::string* result) {
  std::array<unsigned char, 16> nonce{};
  if (getrandom(nonce.data(), nonce.size(), 0) != static_cast<ssize_t>(nonce.size())) return false;
  constexpr char hex[] = "0123456789abcdef";
  result->clear(); result->reserve(32);
  for (const auto byte : nonce) { result->push_back(hex[byte >> 4]); result->push_back(hex[byte & 15]); }
  return true;
}
bool SameFile(const struct stat& value) {
  return S_ISREG(value.st_mode) && value.st_uid == geteuid() &&
    (value.st_mode & 077) == 0 && value.st_nlink == 1 && value.st_size > 0 &&
    value.st_dev == prepared.identity.st_dev && value.st_ino == prepared.identity.st_ino;
}
class Renderer final : public CefRenderProcessHandler {
 public:
  void OnContextCreated(CefRefPtr<CefBrowser>, CefRefPtr<CefFrame> frame,
                        CefRefPtr<CefV8Context> context) override {
    CEF_REQUIRE_RENDERER_THREAD();
    auto line = CefCommandLine::GetGlobalCommandLine();
    if (context_ || !frame || !frame->IsMain() || !context || !context->IsValid() ||
        !line->HasSwitch(kNonceSwitch) ||
        frame->GetURL() != line->GetSwitchValue(kUrlSwitch)) return;
    context_ = context; frame_ = frame->GetIdentifier().ToString();
    auto message = CefProcessMessage::Create(kReady);
    auto values = message->GetArgumentList();
    values->SetString(0, line->GetSwitchValue(kNonceSwitch)); values->SetInt(1, getpid());
    frame->SendProcessMessage(PID_BROWSER, message);
  }
  void OnContextReleased(CefRefPtr<CefBrowser>, CefRefPtr<CefFrame>,
                         CefRefPtr<CefV8Context> context) override {
    CEF_REQUIRE_RENDERER_THREAD();
    if (context && context_ && context_->IsSame(context)) { context_ = nullptr; frame_.clear(); }
  }
  bool OnProcessMessageReceived(CefRefPtr<CefBrowser>, CefRefPtr<CefFrame> frame,
      CefProcessId process, CefRefPtr<CefProcessMessage> message) override {
    CEF_REQUIRE_RENDERER_THREAD();
    if (!message || message->GetName() != kChallenge) return false;
    auto line = CefCommandLine::GetGlobalCommandLine();
    auto values = message->GetArgumentList();
    auto context = frame ? frame->GetV8Context() : nullptr;
    if (sent_ || process != PID_BROWSER || !frame || !frame->IsMain() || !frame->IsValid() ||
        frame->GetIdentifier().ToString() != frame_ || !context_ || !context_->IsValid() ||
        !context || !context->IsSame(context_) || frame->GetURL() != line->GetSwitchValue(kUrlSwitch) ||
        !values || values->GetSize() != 2 || values->GetType(0) != VTYPE_STRING ||
        values->GetType(1) != VTYPE_STRING || values->GetString(0) != line->GetSwitchValue(kNonceSwitch) ||
        values->GetString(1).length() != 32) return true;
    sent_ = true;
    const auto nonce = line->GetSwitchValue(kNonceSwitch).ToString();
    const auto challenge = values->GetString(1).ToString();
    const char* home = std::getenv("HOME");
    int result = 3, error = 0;
    // PR_GET_SECCOMP is deliberately forbidden by the pinned Chromium renderer
    // sandbox. Only the trusted parent may read kernel Seccomp state from proc.
    if (nonce.size() == 32 && home && home[0] == '/' && strnlen(home, PATH_MAX) < PATH_MAX) {
      const std::string path = std::string(home) + "/.pki/nssdb/key4.db";
      const int file = open(path.c_str(), O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
      error = file < 0 ? errno : 0;
      if (file >= 0) {
        // An unexpected successful open is a failure. Never inspect key bytes.
        close(file); result = 2;
      } else if (error == EACCES || error == EPERM) result = 1;
    }
    auto response = CefProcessMessage::Create(kName);
    auto reply = response->GetArgumentList();
    reply->SetString(0, challenge); reply->SetInt(1, result);
    reply->SetInt(2, getpid()); reply->SetInt(3, error);
    frame->SendProcessMessage(PID_BROWSER, response);
    return true;
  }
 private:
  bool sent_ = false;
  std::string frame_;
  CefRefPtr<CefV8Context> context_;
  IMPLEMENT_REFCOUNTING(Renderer);
};
}

CefRefPtr<CefRenderProcessHandler> CustodyRenderer() { return new Renderer; }
void CustodyChild(CefRefPtr<CefCommandLine> line) {
  if (prepared.descriptor >= 0 && line->GetSwitchValue("type") == "renderer") {
    std::string nonce;
    if (prepared.launches.size() >= 32 || !Nonce(&nonce) || !prepared.launches.insert(nonce).second) {
      prepared.consumed = true; return;
    }
    // Sibling renderer PID namespaces can share the same innermost PID. The
    // launch nonce uniquely binds the parent-owned procfs candidate instead.
    line->AppendSwitchWithValue(kNonceSwitch, nonce);
    line->AppendSwitchWithValue(kUrlSwitch, prepared.url);
  }
}
void CustodyDocument(colossus_cef_tab tab, uint64_t generation, CefRefPtr<CefFrame> frame) {
  CEF_REQUIRE_UI_THREAD();
  if (prepared.descriptor < 0 || prepared.consumed || !frame || !frame->IsMain()) return;
  if (prepared.tab == 0) {
    if (frame->GetURL().ToString() != prepared.url) return;
    prepared.tab = tab; prepared.generation = generation;
  }
  if (prepared.tab != tab || prepared.generation != generation) return;
  prepared.challenge.clear(); prepared.launch_nonce.clear();
  prepared.pending_document = 0; prepared.renderer_pid = 0;
  prepared.frame = frame->GetIdentifier().ToString();
  if (prepared.document == std::numeric_limits<uint64_t>::max()) prepared.consumed = true;
  else {
    ++prepared.document;
    fputs("COLOSSUS_NATIVE_NSS_OPEN_DIAGNOSTIC_V1_DOCUMENT_COMMITTED\n", stderr);
  }
}
bool CustodyMessage(colossus_cef_tab tab, uint64_t generation,
    CefRefPtr<CefBrowser> browser, CefRefPtr<CefFrame> frame,
    CefProcessId process, CefRefPtr<CefProcessMessage> message) {
  CEF_REQUIRE_UI_THREAD();
  if (!message || (message->GetName() != kName && message->GetName() != kReady)) return false;
  auto values = message->GetArgumentList();
  const bool known_ready = message->GetName() == kReady && values && values->GetSize() == 2 &&
    values->GetType(0) == VTYPE_STRING && values->GetString(0).length() == 32 &&
    values->GetType(1) == VTYPE_INT && values->GetInt(1) > 0 &&
    prepared.launches.contains(values->GetString(0).ToString());
  if (known_ready && process == PID_RENDERER && frame && frame->IsMain() &&
      prepared.descriptor >= 0 && !prepared.consumed && prepared.document == 0)
    fputs("COLOSSUS_NATIVE_NSS_OPEN_DIAGNOSTIC_V1_READY_BEFORE_COMMIT\n", stderr);
  Tab* owned = nullptr;
  if (prepared.descriptor < 0 || prepared.consumed || process != PID_RENDERER ||
      !browser || !frame || !frame->IsMain() || !frame->IsValid() ||
      prepared.tab != tab || prepared.generation != generation || prepared.document == 0 ||
      frame->GetIdentifier().ToString() != prepared.frame ||
      Resolve(tab, generation, &owned) != COLOSSUS_CEF_OK ||
      owned->closing || !owned->browser || !owned->browser->GetMainFrame() ||
      browser->GetIdentifier() != owned->browser->GetIdentifier() ||
      frame->GetIdentifier() != owned->browser->GetMainFrame()->GetIdentifier() ||
      frame->GetURL().ToString() != prepared.url) return true;
  if (message->GetName() == kReady) {
    if (!known_ready || !prepared.challenge.empty()) return true;
    if (!Nonce(&prepared.challenge)) { prepared.consumed = true; return true; }
    prepared.launch_nonce = values->GetString(0).ToString();
    prepared.renderer_pid = values->GetInt(1); prepared.pending_document = prepared.document;
    prepared.deadline = std::chrono::steady_clock::now() + std::chrono::seconds(5);
    auto challenge = CefProcessMessage::Create(kChallenge);
    auto arguments = challenge->GetArgumentList();
    arguments->SetString(0, prepared.launch_nonce); arguments->SetString(1, prepared.challenge);
    fputs("COLOSSUS_NATIVE_NSS_OPEN_DIAGNOSTIC_V1_CHALLENGE_SENT\n", stderr);
    frame->SendProcessMessage(PID_RENDERER, challenge);
    return true;
  }
  if (!values || values->GetSize() != 4 || values->GetType(0) != VTYPE_STRING ||
      values->GetType(1) != VTYPE_INT || values->GetType(2) != VTYPE_INT ||
      values->GetType(3) != VTYPE_INT || values->GetString(0).length() != 32 || prepared.challenge.empty() ||
      values->GetString(0).ToString() != prepared.challenge ||
      prepared.document != prepared.pending_document || values->GetInt(2) != prepared.renderer_pid ||
      std::chrono::steady_clock::now() >= prepared.deadline)
    return true;
  prepared.consumed = true;
  struct stat retained{}, current{};
  const bool exists = fstat(prepared.descriptor, &retained) == 0 && SameFile(retained) &&
    lstat(prepared.path.c_str(), &current) == 0 && SameFile(current);
  const int result = values->GetInt(1), error = values->GetInt(3);
  if (result == 2) fputs("COLOSSUS_NATIVE_NSS_OPEN_DIAGNOSTIC_V1_OPEN_SUCCEEDED\n", stderr);
  else if (result != 1 || (error != EACCES && error != EPERM))
    fputs("COLOSSUS_NATIVE_NSS_OPEN_DIAGNOSTIC_V1_OPEN_OTHER_ERROR\n", stderr);
  const auto proof = exists && result == 1 && (error == EACCES || error == EPERM) ?
    custody_test::AttestDetailed(prepared.browser, prepared.renderer_pid, prepared.launch_nonce, prepared.url) :
    custody_test::Attestation::InvalidBinding;
  switch (proof) {
    case custody_test::Attestation::ProcUnavailable:
      fputs("COLOSSUS_NATIVE_NSS_OPEN_DIAGNOSTIC_V1_PROC_UNAVAILABLE\n", stderr); break;
    case custody_test::Attestation::NotFound:
      fputs("COLOSSUS_NATIVE_NSS_OPEN_DIAGNOSTIC_V1_PROCESS_NOT_FOUND\n", stderr); break;
    case custody_test::Attestation::Ambiguous:
      fputs("COLOSSUS_NATIVE_NSS_OPEN_DIAGNOSTIC_V1_PROCESS_AMBIGUOUS\n", stderr); break;
    case custody_test::Attestation::SandboxUnverified:
      fputs("COLOSSUS_NATIVE_NSS_OPEN_DIAGNOSTIC_V1_SANDBOX_UNVERIFIED\n", stderr); break;
    case custody_test::Attestation::MetadataUnavailable:
      fputs("COLOSSUS_NATIVE_NSS_OPEN_DIAGNOSTIC_V1_EXE_UNVERIFIED\n", stderr); break;
    case custody_test::Attestation::PidfdUnavailable:
      fputs("COLOSSUS_NATIVE_NSS_OPEN_DIAGNOSTIC_V1_PIDFD_UNAVAILABLE\n", stderr); break;
    case custody_test::Attestation::Unstable:
      fputs("COLOSSUS_NATIVE_NSS_OPEN_DIAGNOSTIC_V1_PROCESS_UNSTABLE\n", stderr); break;
    default: break;
  }
  const bool attested = proof == custody_test::Attestation::Verified;
  const bool current_document = Resolve(tab, generation, &owned) == COLOSSUS_CEF_OK &&
    !owned->closing && prepared.document == prepared.pending_document && frame->IsValid() &&
    frame->GetIdentifier().ToString() == prepared.frame && frame->GetURL().ToString() == prepared.url &&
    owned->browser && owned->browser->GetIdentifier() == browser->GetIdentifier() &&
    owned->browser->GetMainFrame() &&
    owned->browser->GetMainFrame()->GetIdentifier() == frame->GetIdentifier();
  const bool still_exists = fstat(prepared.descriptor, &retained) == 0 && SameFile(retained) &&
    lstat(prepared.path.c_str(), &current) == 0 && SameFile(current);
  if (!current_document) fputs("COLOSSUS_NATIVE_NSS_OPEN_DIAGNOSTIC_V1_DOCUMENT_UNVERIFIED\n", stderr);
  if (!exists || !still_exists) fputs("COLOSSUS_NATIVE_NSS_OPEN_DIAGNOSTIC_V1_INODE_UNVERIFIED\n", stderr);
  const uint8_t category = attested && current_document && still_exists ? 1 : 2;
  // Fixed native-only event. No inode, filename, capability, or key bytes cross it.
  Emit(tab, generation, 19, 0, category == 1, &category, sizeof(category));
  return true;
}
void CustodyFinish() {
  if (prepared.descriptor >= 0) close(prepared.descriptor);
  prepared = {};
}
}

extern "C" int32_t colossus_cef_custody_test_prepare(const char* fixture_url) {
  using namespace colossus;
  if (initialized || prepared.descriptor >= 0 || !fixture_url ||
      strnlen(fixture_url, 4097) > 4096) return COLOSSUS_CEF_INVALID;
  CefURLParts parts;
  if (!CefParseURL(fixture_url, parts) || CefString(&parts.scheme) != "https" ||
      parts.username.length || parts.password.length || parts.fragment.length)
    return COLOSSUS_CEF_INVALID;
  const char* home = std::getenv("HOME");
  if (!home || home[0] != '/' || strnlen(home, PATH_MAX) >= PATH_MAX) return COLOSSUS_CEF_INVALID;
  char canonical[PATH_MAX];
  struct stat parent{};
  if (!realpath(home, canonical) || std::strcmp(home, canonical) != 0 ||
      lstat(home, &parent) != 0 || !S_ISDIR(parent.st_mode) ||
      parent.st_uid != geteuid() || (parent.st_mode & 077) != 0) return COLOSSUS_CEF_DENIED;
  const std::string path = std::string(home) + "/.pki/nssdb/key4.db";
  const int file = open(path.c_str(), O_RDONLY | O_CLOEXEC | O_NOFOLLOW);
  struct stat identity{};
  if (file < 0) return COLOSSUS_CEF_DENIED;
  if (fstat(file, &identity) != 0 || !S_ISREG(identity.st_mode) ||
      identity.st_uid != geteuid() || (identity.st_mode & 077) != 0 ||
      identity.st_nlink != 1 || identity.st_size <= 0 || identity.st_size > 64 * 1024 * 1024) {
    close(file); return COLOSSUS_CEF_DENIED;
  }
  custody_test::BrowserIdentity browser;
  if (!custody_test::Capture(&browser)) {
    close(file); return COLOSSUS_CEF_UNAVAILABLE;
  }
  prepared.descriptor = file; prepared.identity = identity; prepared.path = path;
  prepared.url = fixture_url; prepared.browser = browser;
  return COLOSSUS_CEF_OK;
}
