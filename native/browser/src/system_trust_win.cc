#include "system_trust_win.h"
#include <memory>

namespace colossus {
namespace {
struct CloseStore { void operator()(void* store) const { CertCloseStore(store, 0); } };
struct FreeCertificate {
  void operator()(const CERT_CONTEXT* cert) const { CertFreeCertificateContext(cert); }
};
struct FreeChain {
  void operator()(const CERT_CHAIN_CONTEXT* chain) const { CertFreeCertificateChain(chain); }
};
SystemTrustCategory Category(DWORD errors, DWORD policy) {
  if (errors & CERT_TRUST_IS_REVOKED) return SystemTrustCategory::Revoked;
  if (errors & CERT_TRUST_IS_EXPLICIT_DISTRUST) return SystemTrustCategory::Untrusted;
  if (errors & CERT_TRUST_IS_NOT_SIGNATURE_VALID) return SystemTrustCategory::Invalid;
  if (errors & CERT_TRUST_IS_NOT_TIME_VALID) return SystemTrustCategory::Expired;
  if (errors & CERT_TRUST_IS_NOT_VALID_FOR_USAGE) return SystemTrustCategory::WrongUsage;
  if (errors & (CERT_TRUST_IS_UNTRUSTED_ROOT | CERT_TRUST_IS_PARTIAL_CHAIN))
    return SystemTrustCategory::Untrusted;
  if (policy == CERT_E_CN_NO_MATCH) return SystemTrustCategory::NameMismatch;
  if (errors & (CERT_TRUST_REVOCATION_STATUS_UNKNOWN | CERT_TRUST_IS_OFFLINE_REVOCATION))
    return SystemTrustCategory::RevocationUnknown;
  if (errors || policy) return SystemTrustCategory::Invalid;
  return SystemTrustCategory::VerifiedCached;
}
}

const char* SystemTrustName(SystemTrustCategory category) {
  switch (category) {
    case SystemTrustCategory::VerifiedCached: return "verified_cached";
    case SystemTrustCategory::Untrusted: return "untrusted";
    case SystemTrustCategory::Expired: return "expired";
    case SystemTrustCategory::NameMismatch: return "name_mismatch";
    case SystemTrustCategory::WrongUsage: return "wrong_usage";
    case SystemTrustCategory::Revoked: return "revoked";
    case SystemTrustCategory::RevocationUnknown: return "revocation_unknown";
    case SystemTrustCategory::Invalid: return "invalid";
    default: return "unavailable";
  }
}

SystemTrustDiagnostic DiagnoseSystemServerTrust(
    const std::wstring& hostname,
    const std::vector<std::vector<uint8_t>>& certificates,
    HCERTCHAINENGINE engine, HCERTSTORE additional_store) {
  SystemTrustDiagnostic result;
  if (hostname.empty() || hostname.size() > 253 ||
      hostname.find_first_of(std::wstring(L"\0\r\n\t ", 5)) != std::wstring::npos ||
      certificates.empty() || certificates.size() > 64) return result;
  size_t total = 0;
  for (const auto& certificate : certificates) {
    if (certificate.empty() || certificate.size() > 65536) return result;
    total += certificate.size();
    if (total > 512 * 1024) return result;
  }
  std::unique_ptr<void, CloseStore> store(CertOpenStore(
      CERT_STORE_PROV_MEMORY, 0, 0, CERT_STORE_CREATE_NEW_FLAG, nullptr));
  if (!store) return result;
  for (const auto& certificate : certificates) {
    if (!CertAddEncodedCertificateToStore(store.get(), X509_ASN_ENCODING,
          certificate.data(), static_cast<DWORD>(certificate.size()),
          CERT_STORE_ADD_ALWAYS, nullptr)) return result;
  }
  // Supporting certificates are an untrusted source, never trust anchors.
  std::unique_ptr<void, CloseStore> collection(CertOpenStore(
      CERT_STORE_PROV_COLLECTION, 0, 0, CERT_STORE_CREATE_NEW_FLAG, nullptr));
  if (!collection || !CertAddStoreToCollection(collection.get(), store.get(), 0, 0) ||
      (additional_store &&
       !CertAddStoreToCollection(collection.get(), additional_store, 0, 0))) return result;
  const auto& encoded_leaf = certificates.front();
  std::unique_ptr<const CERT_CONTEXT, FreeCertificate> leaf(CertCreateCertificateContext(
      X509_ASN_ENCODING, encoded_leaf.data(), static_cast<DWORD>(encoded_leaf.size())));
  if (!leaf) return result;
  LPSTR server_auth = const_cast<LPSTR>(szOID_PKIX_KP_SERVER_AUTH);
  CERT_CHAIN_PARA parameters{};
  parameters.cbSize = sizeof(parameters);
  parameters.RequestedUsage.dwType = USAGE_MATCH_TYPE_AND;
  parameters.RequestedUsage.Usage.cUsageIdentifier = 1;
  parameters.RequestedUsage.Usage.rgpszUsageIdentifier = &server_auth;
  constexpr DWORD flags = CERT_CHAIN_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT |
      CERT_CHAIN_REVOCATION_CHECK_CACHE_ONLY | CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL |
      CERT_CHAIN_DISABLE_AUTH_ROOT_AUTO_UPDATE | CERT_CHAIN_DISABLE_AIA;
  PCCERT_CHAIN_CONTEXT raw_chain = nullptr;
  if (!CertGetCertificateChain(engine, leaf.get(), nullptr, collection.get(),
        &parameters, flags, nullptr, &raw_chain)) return result;
  std::unique_ptr<const CERT_CHAIN_CONTEXT, FreeChain> chain(raw_chain);
  SSL_EXTRA_CERT_CHAIN_POLICY_PARA ssl{};
  ssl.cbSize = sizeof(ssl);
  ssl.dwAuthType = AUTHTYPE_SERVER;
  ssl.pwszServerName = const_cast<wchar_t*>(hostname.c_str());
  CERT_CHAIN_POLICY_PARA policy{};
  policy.cbSize = sizeof(policy);
  policy.pvExtraPolicyPara = &ssl;
  CERT_CHAIN_POLICY_STATUS status{};
  status.cbSize = sizeof(status);
  if (!CertVerifyCertificateChainPolicy(CERT_CHAIN_POLICY_SSL, chain.get(), &policy, &status))
    return result;
  result.chain_errors = chain->TrustStatus.dwErrorStatus;
  result.policy_error = status.dwError;
  result.category = Category(result.chain_errors, result.policy_error);
  return result;
}
}
