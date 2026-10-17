#ifndef COLOSSUS_BROWSER_PKI_MAC_OWNED_IDENTITY_H_
#define COLOSSUS_BROWSER_PKI_MAC_OWNED_IDENTITY_H_

#include <array>
#include <chrono>
#include <cstdint>
#include <memory>
#include <span>
#include <string>
#include <vector>

// Private native prototype, never a renderer, browser-action, or public SDK API.
// The CEF host does not link this library. A separate admitted TLS broker must
// own it and run both TLS legs with per-request authority enforcement.
namespace colossus::browser::pki::mac {

enum class Status { Ready, Denied, Unavailable, OutcomeUnknown };
// Fixed Debug diagnostics contain categories/OSStatus integers only. No native
// pointers, paths, certificates, passwords or key material leave the owner.
enum class DiagnosticPhase : std::uint32_t { Admission, Namespace, Create, Import, Validate, Sign, Retire };
enum class DiagnosticCheck : std::uint32_t {
  Start, NativeApi, Path, Metadata, Entries, Harden, Seal, Binding,
  StoreStatus, StoreLocked, Code, Access, ImportShape, Leaf, Key, Acl,
  Items, Parent, Window, Lock, Rename, RemoveStore, RemoveAuxiliary,
  RemoveDirectory, Sync, Unlock
};
struct Diagnostic {
  DiagnosticPhase phase = DiagnosticPhase::Admission;
  DiagnosticCheck check = DiagnosticCheck::Start;
  std::int32_t os_status = 0;
};
using Fingerprint = std::array<std::uint8_t, 32>;
using CodeHash = std::array<std::uint8_t, 20>;
using Generation = std::array<std::uint8_t, 32>;

struct Binding {
  std::string origin;
  Fingerprint leaf_sha256{};
};

// Mandatory callbacks reuse native/browser/pki's effect-free Rust validators.
// They must reject malformed, stale, CA/wrong-usage client leaves and noncanonical
// HTTPS origins. No callback is sourced from external IPC or browser content.
struct PublicValidation {
  bool (*ca)(std::span<const std::uint8_t>) = nullptr;
  bool (*identity)(std::span<const std::uint8_t>) = nullptr;
  bool (*origin)(const std::string&) = nullptr;
};

enum class Mutation { CreateStore, ImportIdentity, RetireStore, CreateNamespace };

// In-process native supervisor boundary, never caller-asserted IPC fields. The
// implementation independently excludes every other process/filesystem grant
// from this retained namespace for the whole window. An advisory file lock or
// mode0700 alone is insufficient. Failure keeps the physical state unaccepted.
class MutationWindow {
 public:
  virtual ~MutationWindow() = default;
  virtual bool Exclusive() const = 0;
  // Called only after a successful native operation, with the exact open output
  // descriptors (one directory for namespace creation, two files for store or
  // import). The independent owner seals their generation before adoption.
  virtual bool Seal(std::uint64_t sequence, std::span<const int> output_fds) = 0;
  // Positively verified ordered physical retirement progress, persisted before
  // publication. Failure keeps a real retained cleanup obligation.
  virtual bool Checkpoint(std::uint64_t sequence, std::uint32_t step) = 0;
  // A failed import may leave unknown contents inside the SAME already owned
  // physical store. Quarantine/retire it only after independent exact-descriptor
  // proof; never adopt a new or replaced output from a failed native operation.
  virtual bool RetireUnchanged(std::uint64_t sequence, std::span<const int> previous_fds) = 0;
};
class MutationAuthority {
 public:
  virtual ~MutationAuthority() = default;
  // Required current parent/package/generation admission, including before and
  // after signing. Retirement may remain admitted after parent loss.
  virtual bool SigningAdmitted(const Generation& generation) const = 0;
  virtual std::unique_ptr<MutationWindow> Begin(
      const Generation& generation, int retained_parent, int retained_directory,
      std::uint64_t sequence, Mutation operation) = 0;
};

struct Bootstrap {
  // The native supervisor retains this newly created private parent outside
  // EVERY Root/browser filesystem grant. Mode0700 alone is not that boundary.
  // Passing these fields does not prove process or filesystem admission.
  int retained_parent_fd = -1;
  std::string canonical_parent;
  CodeHash expected_broker_cdhash{};
  // Fresh supervisor entropy for this enrollment. Every broker request carries
  // it so a command retained from an earlier store generation fails closed.
  Generation generation{};
  std::vector<Binding> bindings;
  PublicValidation validate;
  std::shared_ptr<MutationAuthority> mutations;
};

struct Enrollment {
  // Authenticated native bootstrap bytes only; never renderer/model paths.
  std::vector<std::vector<std::uint8_t>> ca_der;
  std::vector<std::uint8_t> reviewed_leaf_der;
  std::span<const std::uint8_t> encrypted_pkcs12;
  // Nonempty printable ASCII, at most 128 bytes. Wiped on every Prepare exit.
  std::span<std::uint8_t> password;
};

enum class Tls13Scheme : std::uint16_t {
  EcdsaP256Sha256 = 0x0403,
  RsaPssSha256 = 0x0804,
};

// A local, one-use broker handshake token. It never crosses browser/Root IPC.
// It is useful only inside this owner and before its bounded deadline.
struct Handshake {
  std::uint64_t sequence = 0;
  Generation generation{};
};

class OwnedIdentity final {
 public:
  OwnedIdentity(const OwnedIdentity&) = delete;
  OwnedIdentity& operator=(const OwnedIdentity&) = delete;
  ~OwnedIdentity();

  // On a partial store outcome, output retains the physical cleanup obligation.
  // Never discard it or recursively prune the parent on a non-Ready result.
  static Status Prepare(Bootstrap bootstrap, Enrollment enrollment,
                        std::unique_ptr<OwnedIdentity>* output);

  // Public material only. CA roots belong in the broker's own TLS verifier;
  // importing a client chain grants no CA trust. No OS trust setter is used.
  Status CopyPublicMaterial(const Generation& generation,
                            std::vector<std::vector<std::uint8_t>>* ca_der,
                            std::vector<std::uint8_t>* leaf_der);

  // Native TLS engine only, after the exact admitted request origin/lease check.
  // This is not an IPC signing oracle. Only one outstanding handshake is kept.
  Status BeginHandshake(const Generation& generation,
                        const std::string& admitted_origin,
                        const Fingerprint& reviewed_leaf,
                        std::chrono::steady_clock::time_point deadline,
                        Handshake* output);
  // Fixed RFC8446 client CertificateVerify construction, SHA256 transcript only.
  // No generic digest/raw-message/private-key/export method exists.
  Status CertificateVerify(Handshake handshake, Tls13Scheme scheme,
                           const std::array<std::uint8_t, 32>& transcript_sha256,
                           std::vector<std::uint8_t>* signature);
  void CancelHandshake(Handshake handshake);

  // Revoke/drain before cleanup. Unknown namespace identity remains preserved.
  void Revoke();
  Status Finish();
  Diagnostic CopyDiagnostic();

 private:
#if defined(COLOSSUS_MAC_OWNED_PKI_FILE_ONLY_TEST)
  friend struct FileOnlyFixture;
#endif
  struct State;
  explicit OwnedIdentity(std::unique_ptr<State> state);
  std::unique_ptr<State> state_;
};
}  // namespace colossus::browser::pki::mac
#endif
