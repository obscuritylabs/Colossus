#include "system_trust_win.h"
#include <cstdio>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <vector>

namespace {
void CheckAt(bool condition, int line) {
  if (!condition) {
    std::fprintf(stderr, "FAIL native system trust fixture line=%d error=%lu\n", line, GetLastError());
    std::abort();
  }
}
#define Check(condition) CheckAt((condition), __LINE__)
std::vector<uint8_t> Read(const std::filesystem::path& path) {
  std::ifstream input(path, std::ios::binary);
  Check(input.good());
  std::vector<uint8_t> bytes{std::istreambuf_iterator<char>(input), {}};
  Check(!bytes.empty() && bytes.size() < 65536);
  return bytes;
}
struct Store {
  HCERTSTORE handle = CertOpenStore(CERT_STORE_PROV_MEMORY, 0, 0,
                                    CERT_STORE_CREATE_NEW_FLAG, nullptr);
  Store() { Check(handle != nullptr); }
  ~Store() { CertCloseStore(handle, CERT_CLOSE_STORE_CHECK_FLAG); }
  void Certificate(const std::vector<uint8_t>& bytes) const {
    Check(CertAddEncodedCertificateToStore(handle, X509_ASN_ENCODING, bytes.data(),
        static_cast<DWORD>(bytes.size()), CERT_STORE_ADD_ALWAYS, nullptr) != FALSE);
  }
  void Crl(const std::vector<uint8_t>& bytes) const {
    Check(CertAddEncodedCRLToStore(handle, X509_ASN_ENCODING, bytes.data(),
        static_cast<DWORD>(bytes.size()), CERT_STORE_ADD_ALWAYS, nullptr) != FALSE);
  }
};
struct Engine {
  HCERTCHAINENGINE handle = nullptr;
  Engine(const Store& roots, const Store& support, const Store& empty) {
    CERT_CHAIN_ENGINE_CONFIG config{};
    config.cbSize = sizeof(config);
    config.hExclusiveRoot = roots.handle;
    config.hExclusiveTrustedPeople = empty.handle;
    config.cAdditionalStore = 1;
    HCERTSTORE extra = support.handle;
    config.rghAdditionalStore = &extra;
    config.dwFlags = CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL | CERT_CHAIN_DISABLE_AIA;
    Check(CertCreateCertificateChainEngine(&config, &handle) != FALSE);
  }
  ~Engine() { CertFreeCertificateChainEngine(handle); }
};
}

int wmain(int argc, wchar_t** argv) {
  Check(argc == 2);
  const std::filesystem::path directory(argv[1]);
  Store roots, support, empty;
  const auto root = Read(directory / L"root.der");
  const auto intermediate = Read(directory / L"intermediate.der");
  const auto leaf = Read(directory / L"valid.der");
  roots.Certificate(root);
  support.Certificate(intermediate);
  support.Crl(Read(directory / L"root.crl"));
  support.Crl(Read(directory / L"intermediate.crl"));
  Engine engine(roots, support, empty);
  using colossus::SystemTrustCategory;
  auto expect = [&](const char* name, const std::vector<uint8_t>& certificate,
                    const std::wstring& hostname, SystemTrustCategory expected) {
    const auto result = colossus::DiagnoseSystemServerTrust(
        hostname, {certificate, intermediate}, engine.handle, support.handle);
    if (result.category != expected) {
      std::fprintf(stderr, "FAIL native trust case=%s category=%s chain=%lu policy=%lu\n",
          name, colossus::SystemTrustName(result.category), result.chain_errors, result.policy_error);
      std::abort();
    }
    std::printf("PASS native trust case=%s category=%s\n", name, colossus::SystemTrustName(result.category));
  };
  expect("trusted", leaf, L"pki-fixture.invalid", SystemTrustCategory::VerifiedCached);
  expect("untrusted", Read(directory / L"untrusted.der"), L"pki-fixture.invalid", SystemTrustCategory::Untrusted);
  expect("hostname", leaf, L"other.invalid", SystemTrustCategory::NameMismatch);
  expect("expired", Read(directory / L"expired.der"), L"pki-fixture.invalid", SystemTrustCategory::Expired);
  expect("revoked", Read(directory / L"revoked.der"), L"pki-fixture.invalid", SystemTrustCategory::Revoked);
  expect("usage", Read(directory / L"wrong_usage.der"), L"pki-fixture.invalid", SystemTrustCategory::WrongUsage);
  auto corrupted = leaf;
  corrupted.back() ^= 1;
  expect("signature", corrupted, L"pki-fixture.invalid", SystemTrustCategory::Invalid);

  Store uncached;
  uncached.Certificate(intermediate);
  Engine unknown(roots, uncached, empty);
  const auto result = colossus::DiagnoseSystemServerTrust(
      L"pki-fixture.invalid", {leaf, intermediate}, unknown.handle, uncached.handle);
  Check(result.category == SystemTrustCategory::RevocationUnknown);
  std::puts("PASS native trust case=uncached_revocation category=revocation_unknown");
  Check(colossus::DiagnoseSystemServerTrust(L"", {leaf}, engine.handle).category == SystemTrustCategory::Unavailable);
  Check(colossus::DiagnoseSystemServerTrust(L"pki-fixture.invalid", {{0}}, engine.handle).category == SystemTrustCategory::Unavailable);
  // Read-only visibility is separate evidence from exclusive fixture validation.
  // Never add to, remove from, or enumerate the operator's personal stores.
  for (const DWORD location : {CERT_SYSTEM_STORE_CURRENT_USER, CERT_SYSTEM_STORE_LOCAL_MACHINE}) {
    for (const auto name : {L"ROOT", L"CA", L"Disallowed"}) {
      const auto store = CertOpenStore(CERT_STORE_PROV_SYSTEM_W, 0, 0,
          location | CERT_STORE_READONLY_FLAG | CERT_STORE_OPEN_EXISTING_FLAG, name);
      Check(store != nullptr);
      CertCloseStore(store, 0);
    }
  }
  std::puts("PASS current-user and machine certificate stores readable without mutation");
  return 0;
}
