#ifndef COLOSSUS_BROWSER_SYSTEM_TRUST_WIN_H
#define COLOSSUS_BROWSER_SYSTEM_TRUST_WIN_H

#include <windows.h>
#include <wincrypt.h>
#include <cstdint>
#include <string>
#include <vector>

namespace colossus {
enum class SystemTrustCategory {
  VerifiedCached, Untrusted, Expired, NameMismatch, WrongUsage, Revoked,
  RevocationUnknown, Invalid, Unavailable
};
struct SystemTrustDiagnostic {
  SystemTrustCategory category = SystemTrustCategory::Unavailable;
  DWORD chain_errors = 0;
  DWORD policy_error = 0;
};
const char* SystemTrustName(SystemTrustCategory category);

// Diagnostics only: a successful Windows check MUST NOT override Chromium's
// certificate decision. Uses the caller's existing OS trust and cached revocation
// data, with no AIA, CRL, OCSP or automatic-root network retrieval. The optional
// engine/store allow exclusive in-memory native fixtures; production uses neither.
SystemTrustDiagnostic DiagnoseSystemServerTrust(
    const std::wstring& hostname,
    const std::vector<std::vector<uint8_t>>& leaf_and_issuers,
    HCERTCHAINENGINE engine = HCCE_CURRENT_USER,
    HCERTSTORE additional_store = nullptr);
}
#endif
