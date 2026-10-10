#include "owned_identity.h"
#if !defined(COLOSSUS_MAC_OWNED_PKI_PROTOTYPE) || defined(NDEBUG)
#error "Owned macOS PKI is an explicit, unaccepted Debug broker prototype"
#endif
#import <Security/Security.h>
#include <CommonCrypto/CommonDigest.h>
#include <sys/acl.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <dirent.h>
#include <limits.h>
#include <algorithm>
#include <cerrno>
#include <cstring>
#include <mutex>
#include <optional>

// Public legacy file-Keychain APIs only. No weak private SPI, Security item
// interposition, default/search-list, OS trust setter, or key export operation.
// They remain unaccepted until a separate broker and actual denial/TLS fixtures
// bind this exact source and component to its native containment admission.
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"

namespace colossus::browser::pki::mac {
namespace {
constexpr char kDirectory[] = "colossus-tls-identity";
constexpr char kStore[] = "colossus-tls-identity.keychain";
constexpr std::size_t kMaxPkcs12 = 4 * 1024 * 1024;
constexpr std::size_t kMaxRoots = 8;
constexpr std::size_t kMaxRootBytes = 1024 * 1024;
constexpr std::size_t kMaxLeafBytes = 65536;

template <class T> struct Ref {
  T value = nullptr;
  Ref() = default;
  Ref(const Ref&) = delete;
  Ref& operator=(const Ref&) = delete;
  ~Ref() { if (value) CFRelease(value); }
  T* out() { return &value; }
  T release() { T result = value; value = nullptr; return result; }
};
void wipe(std::span<std::uint8_t> bytes) {
  volatile std::uint8_t* value = bytes.data();
  for (std::size_t index = 0; index < bytes.size(); ++index) value[index] = 0;
}
struct WipeOnExit {
  std::span<std::uint8_t> bytes;
  ~WipeOnExit() { wipe(bytes); }
};
struct FileIdentity {
  dev_t device{};
  ino_t inode{};
  uid_t uid{};
  static FileIdentity From(const struct stat& value) {
    return {value.st_dev, value.st_ino, value.st_uid};
  }
  bool Same(const struct stat& value) const {
    return device == value.st_dev && inode == value.st_ino && uid == value.st_uid;
  }
};
bool private_metadata(const struct stat& value, bool directory) {
  return value.st_uid == geteuid() && !(value.st_mode & 0077) &&
      (directory ? S_ISDIR(value.st_mode) : S_ISREG(value.st_mode) && value.st_nlink == 1);
}
bool no_acl(int descriptor) {
  filesec_t security = filesec_init();
  if (!security) return false;
  struct stat metadata{};
  int present = 0;
  bool empty = false;
  if (fstatx_np(descriptor, &metadata, security) == 0 &&
      filesec_query_property(security, FILESEC_ACL, &present) == 0) {
    if (!present) empty = true;
    else {
      acl_t acl = nullptr;
      if (filesec_get_property(security, FILESEC_ACL, &acl) == 0 && acl) {
        acl_entry_t entry{};
        errno = 0;
        const int result = acl_get_entry(acl, ACL_FIRST_ENTRY, &entry);
        const int error = errno;
        empty = acl_valid(acl) == 0 && result == -1 && error == EINVAL;
        acl_free(acl);
      }
    }
  }
  filesec_free(security);
  return empty;
}
bool canonical(const std::string& path) {
  char resolved[PATH_MAX];
  return !path.empty() && path.front() == '/' && path.size() < PATH_MAX &&
      path.find('\0') == std::string::npos && realpath(path.c_str(), resolved) && path == resolved;
}
bool forbidden_parent(const std::string& path) {
  std::string folded = path;
  for (char& byte : folded) if (byte >= 'A' && byte <= 'Z') byte += ('a' - 'A');
  // File-Keychain creation has special login-path search-list behavior. Never
  // allow a parent that can make our fixed child store match that namespace.
  return folded.find("/login.keychain") != std::string::npos ||
      folded.find("/system.keychain") != std::string::npos;
}
bool keychain_path(SecKeychainRef keychain, const std::string& expected) {
  char path[PATH_MAX];
  UInt32 length = sizeof(path);
  return keychain && SecKeychainGetPath(keychain, &length, path) == errSecSuccess &&
      length < sizeof(path) && expected == std::string(path, length);
}
Fingerprint fingerprint(std::span<const std::uint8_t> bytes) {
  Fingerprint result{};
  CC_SHA256(bytes.data(), static_cast<CC_LONG>(bytes.size()), result.data());
  return result;
}
bool nonzero(const CodeHash& value) {
  return std::any_of(value.begin(), value.end(), [](auto byte) { return byte != 0; });
}
bool nonzero(const Generation& value) {
  return std::any_of(value.begin(), value.end(), [](auto byte) { return byte != 0; });
}
bool boolean(CFDictionaryRef dictionary, CFStringRef attribute, bool expected) {
  CFTypeRef value = CFDictionaryGetValue(dictionary, attribute);
  return value && CFGetTypeID(value) == CFBooleanGetTypeID() &&
      CFBooleanGetValue(static_cast<CFBooleanRef>(value)) == expected;
}
}  // namespace

struct OwnedIdentity::State {
  enum class Cleanup { Live, Locked, Released, Retired, FileRemoved, DirectoryRemoved, Finished };
  std::mutex mutex;
  Bootstrap bootstrap;
  std::vector<std::vector<std::uint8_t>> roots;
  std::vector<std::uint8_t> leaf;
  Fingerprint leaf_hash{};
  bool ready = false, changed = false, created_directory = false;
  bool creation_unknown = false, store_recorded = false;
  Cleanup cleanup = Cleanup::Live;
  int parent = -1, directory = -1, file = -1;
  FileIdentity parent_id{}, directory_id{}, file_id{};
  std::string directory_path, store_path, store_name, retired_name;
  SecKeychainRef keychain = nullptr;
  CFArrayRef imported = nullptr;
  SecIdentityRef identity = nullptr;
  SecKeyRef key = nullptr;
  SecCodeRef code = nullptr;
  SecRequirementRef requirement = nullptr;
  CFDataRef trusted_data = nullptr;
  std::uint64_t sequence = 0;
  std::optional<std::pair<std::uint64_t, std::chrono::steady_clock::time_point>> handshake;

  ~State() {
    // No destructor/atexit store mutation. Unknown physical state is retained
    // for the supervising owner; an early broker drop cannot certify cleanup.
    RetireGeneration();
    ReleaseItems();
    if (keychain) CFRelease(keychain);
    if (code) CFRelease(code);
    if (requirement) CFRelease(requirement);
    if (trusted_data) CFRelease(trusted_data);
    for (int descriptor : {file, directory, parent}) if (descriptor >= 0) close(descriptor);
  }
  void ReleaseItems() {
    if (key) { CFRelease(key); key = nullptr; }
    if (identity) { CFRelease(identity); identity = nullptr; }
    if (imported) { CFRelease(imported); imported = nullptr; }
  }
  bool ParentValid() const {
    struct stat held{}, entry{};
    return parent >= 0 && fstat(parent, &held) == 0 && parent_id.Same(held) &&
        private_metadata(held, true) && no_acl(parent) &&
        lstat(bootstrap.canonical_parent.c_str(), &entry) == 0 && parent_id.Same(entry) &&
        private_metadata(entry, true) && canonical(bootstrap.canonical_parent);
  }
  std::string CurrentName() const {
    return cleanup >= Cleanup::Retired ? retired_name : kDirectory;
  }
  bool NamespaceValid() const {
    struct stat held{}, entry{};
    const std::string name = CurrentName();
    return ParentValid() && directory >= 0 && !name.empty() &&
        fstat(directory, &held) == 0 && directory_id.Same(held) &&
        private_metadata(held, true) && no_acl(directory) &&
        fstatat(parent, name.c_str(), &entry, AT_SYMLINK_NOFOLLOW) == 0 && directory_id.Same(entry) &&
        private_metadata(entry, true) && canonical(bootstrap.canonical_parent + "/" + name);
  }
  bool FileBound() const {
    struct stat held{}, entry{};
    return store_recorded && file >= 0 && NamespaceValid() && fstat(file, &held) == 0 &&
        file_id.Same(held) && private_metadata(held, false) && no_acl(file) &&
        fstatat(directory, store_name.c_str(), &entry, AT_SYMLINK_NOFOLLOW) == 0 &&
        file_id.Same(entry) && private_metadata(entry, false) &&
        canonical(bootstrap.canonical_parent + "/" + CurrentName() + "/" + store_name);
  }
  bool ExactEntries(bool expect_store) const {
    const int copy = fcntl(directory, F_DUPFD_CLOEXEC, 3);
    if (copy < 0) return false;
    DIR* entries = fdopendir(copy);
    if (!entries) { close(copy); return false; }
    rewinddir(entries);
    unsigned int count = 0;
    bool valid = true;
    while (true) {
      errno = 0;
      dirent* entry = readdir(entries);
      if (!entry) { if (errno) valid = false; break; }
      const std::string name = entry->d_name;
      if (name == "." || name == "..") continue;
      struct stat value{};
      if (!expect_store || name != store_name || ++count > 1 ||
          fstatat(directory, name.c_str(), &value, AT_SYMLINK_NOFOLLOW) != 0 ||
          !file_id.Same(value) || !private_metadata(value, false)) { valid = false; break; }
    }
    closedir(entries);
    return valid && count == (expect_store ? 1u : 0u);
  }
  bool StoreBound() const {
    return cleanup == Cleanup::Live && FileBound() && ExactEntries(true) &&
        keychain_path(keychain, store_path);
  }
  bool StoreValid() const {
    SecKeychainStatus status = 0;
    return StoreBound() && SecKeychainGetStatus(keychain, &status) == errSecSuccess && (status & kSecUnlockStateStatus);
  }
  bool BindStore() {
    char path[PATH_MAX];
    UInt32 length = sizeof(path);
    if (!keychain || SecKeychainGetPath(keychain, &length, path) != errSecSuccess || length >= sizeof(path)) return false;
    store_path.assign(path, length);
    const std::string expected = directory_path + "/" + kStore;
    if (store_path != expected && store_path != expected + "-db") return false;
    store_name = store_path.substr(directory_path.size() + 1);
    file = openat(directory, store_name.c_str(), O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
    struct stat value{};
    if (file < 0 || fstat(file, &value) != 0 || !private_metadata(value, false) || !no_acl(file)) return false;
    file_id = FileIdentity::From(value);
    store_recorded = true;
    return StoreValid();
  }
  bool CaptureCode() {
    Ref<SecCodeRef> current;
    Ref<SecStaticCodeRef> disk;
    Ref<CFDictionaryRef> information;
    if (SecCodeCopySelf(kSecCSDefaultFlags, current.out()) != errSecSuccess ||
        SecCodeCopyStaticCode(current.value, kSecCSDefaultFlags, disk.out()) != errSecSuccess ||
        SecCodeCopySigningInformation(disk.value, kSecCSSigningInformation, information.out()) != errSecSuccess) return false;
    CFTypeRef hash = CFDictionaryGetValue(information.value, kSecCodeInfoUnique);
    if (!hash || CFGetTypeID(hash) != CFDataGetTypeID() ||
        CFDataGetLength(static_cast<CFDataRef>(hash)) != static_cast<CFIndex>(bootstrap.expected_broker_cdhash.size()) ||
        memcmp(CFDataGetBytePtr(static_cast<CFDataRef>(hash)), bootstrap.expected_broker_cdhash.data(), bootstrap.expected_broker_cdhash.size())) return false;
    constexpr char hex[] = "0123456789abcdef";
    std::string expression = "cdhash H\"";
    for (auto byte : bootstrap.expected_broker_cdhash) { expression += hex[byte >> 4]; expression += hex[byte & 15]; }
    expression += '"';
    Ref<CFStringRef> text;
    text.value = CFStringCreateWithCString(kCFAllocatorDefault, expression.c_str(), kCFStringEncodingASCII);
    Ref<SecRequirementRef> exact;
    if (!text.value || SecRequirementCreateWithString(text.value, kSecCSDefaultFlags, exact.out()) != errSecSuccess ||
        SecCodeCheckValidity(current.value, kSecCSDefaultFlags, exact.value) != errSecSuccess) return false;
    code = current.release();
    requirement = exact.release();
    return true;
  }
  bool CodeValid() const {
    return code && requirement && SecCodeCheckValidity(code, kSecCSDefaultFlags, requirement) == errSecSuccess;
  }
  bool BindingValid(const Generation& generation, const std::string& origin,
                    const Fingerprint& leaf) const {
    return generation == bootstrap.generation && leaf == leaf_hash &&
        bootstrap.validate.origin && bootstrap.validate.origin(origin) &&
        std::any_of(bootstrap.bindings.begin(), bootstrap.bindings.end(),
                    [&origin, &leaf](const auto& binding) {
                      return binding.origin == origin && binding.leaf_sha256 == leaf;
                    });
  }
  void RetireGeneration() {
    wipe(bootstrap.generation);
  }
  bool SameStore(SecKeychainItemRef item) const {
    Ref<SecKeychainRef> store;
    return item && SecKeychainItemCopyKeychain(item, store.out()) == errSecSuccess &&
        keychain_path(store.value, store_path);
  }
  bool KeyProperties() const {
    const CSSM_KEY* header = nullptr;
    if (!key || SecKeyGetCSSMKey(key, &header) != errSecSuccess || !header ||
        header->KeyHeader.BlobType != CSSM_KEYBLOB_REFERENCE ||
        (header->KeyHeader.KeyAttr & CSSM_KEYATTR_EXTRACTABLE) ||
        !(header->KeyHeader.KeyAttr & CSSM_KEYATTR_SENSITIVE) ||
        !(header->KeyHeader.KeyAttr & CSSM_KEYATTR_PERMANENT) ||
        header->KeyHeader.KeyUsage != CSSM_KEYUSE_SIGN) return false;
    // Inspect flags only; never read/serialize CSSM KeyData or an export.
    Ref<CFDictionaryRef> attributes;
    attributes.value = SecKeyCopyAttributes(key);
    return attributes.value && boolean(attributes.value, kSecAttrCanSign, true) &&
        boolean(attributes.value, kSecAttrCanDecrypt, false) &&
        boolean(attributes.value, kSecAttrCanDerive, false);
  }
  bool AclValid() const {
    Ref<SecAccessRef> access;
    Ref<CFArrayRef> list;
    if (!key || !trusted_data ||
        SecKeychainItemCopyAccess(reinterpret_cast<SecKeychainItemRef>(key), access.out()) != errSecSuccess ||
        SecAccessCopyACLList(access.value, list.out()) != errSecSuccess || !list.value ||
        CFArrayGetCount(list.value) == 0 || CFArrayGetCount(list.value) > 32) return false;
    for (CFIndex index = 0; index < CFArrayGetCount(list.value); ++index) {
      Ref<CFArrayRef> applications;
      Ref<CFStringRef> description;
      SecKeychainPromptSelector prompt = 0;
      if (SecACLCopyContents(static_cast<SecACLRef>(const_cast<void*>(CFArrayGetValueAtIndex(list.value, index))),
                            applications.out(), description.out(), &prompt) != errSecSuccess ||
          !applications.value || CFArrayGetCount(applications.value) != 1) return false;
      CFTypeRef application = CFArrayGetValueAtIndex(applications.value, 0);
      Ref<CFDataRef> data;
      if (!application || CFGetTypeID(application) != SecTrustedApplicationGetTypeID() ||
          SecTrustedApplicationCopyData(static_cast<SecTrustedApplicationRef>(const_cast<void*>(application)), data.out()) != errSecSuccess ||
          !data.value || !CFEqual(data.value, trusted_data)) return false;
    }
    // Public CopyData proves the path representation, not an exact serialized
    // code requirement. Cross-process ACL/code-denial acceptance stays pending.
    return true;
  }
  bool Valid() {
    if (!ready || changed || cleanup != Cleanup::Live) return false;
    if (!CodeValid() || !StoreBound() || !SameStore(reinterpret_cast<SecKeychainItemRef>(key)) ||
        !KeyProperties() || !AclValid()) {
      ready = false;
      changed = true;
      handshake.reset();
      return false;
    }
    // A locked owned store or an expired public certificate revokes signing;
    // neither changes the positively retained physical cleanup identity.
    if (!StoreValid() || !bootstrap.validate.identity(leaf)) {
      ready = false;
      handshake.reset();
      return false;
    }
    return true;
  }
};

OwnedIdentity::OwnedIdentity(std::unique_ptr<State> state) : state_(std::move(state)) {}
OwnedIdentity::~OwnedIdentity() = default;

Status OwnedIdentity::Prepare(Bootstrap bootstrap, Enrollment enrollment,
                             std::unique_ptr<OwnedIdentity>* output) {
  WipeOnExit password_wipe{enrollment.password};
  if (!output || *output || bootstrap.retained_parent_fd < 0 ||
      !canonical(bootstrap.canonical_parent) || forbidden_parent(bootstrap.canonical_parent) ||
      !nonzero(bootstrap.expected_broker_cdhash) || !nonzero(bootstrap.generation) ||
      !bootstrap.validate.ca || !bootstrap.validate.identity || !bootstrap.validate.origin ||
      bootstrap.bindings.empty() || bootstrap.bindings.size() > 32 ||
      enrollment.encrypted_pkcs12.empty() || enrollment.encrypted_pkcs12.size() > kMaxPkcs12 ||
      enrollment.password.empty() || enrollment.password.size() > 128 ||
      !std::all_of(enrollment.password.begin(), enrollment.password.end(), [](auto byte) { return byte >= 32 && byte <= 126; }) ||
      enrollment.reviewed_leaf_der.empty() || enrollment.reviewed_leaf_der.size() > kMaxLeafBytes ||
      !bootstrap.validate.identity(enrollment.reviewed_leaf_der) || enrollment.ca_der.size() > kMaxRoots) return Status::Denied;
  std::size_t root_bytes = 0;
  for (const auto& root : enrollment.ca_der) {
    if (root.empty() || root.size() > kMaxRootBytes - root_bytes || !bootstrap.validate.ca(root)) return Status::Denied;
    root_bytes += root.size();
  }
  const Fingerprint leaf_hash = fingerprint(enrollment.reviewed_leaf_der);
  for (std::size_t index = 0; index < bootstrap.bindings.size(); ++index) {
    const auto& binding = bootstrap.bindings[index];
    if (binding.origin.empty() || binding.origin.size() > 4096 ||
        !bootstrap.validate.origin(binding.origin) || binding.leaf_sha256 != leaf_hash ||
        std::any_of(bootstrap.bindings.begin(), bootstrap.bindings.begin() + index,
                    [&binding](const auto& other) { return other.origin == binding.origin; })) return Status::Denied;
  }
  auto state = std::make_unique<State>();
  state->bootstrap = std::move(bootstrap);
  state->roots = std::move(enrollment.ca_der);
  state->leaf = std::move(enrollment.reviewed_leaf_der);
  state->leaf_hash = leaf_hash;
  state->parent = fcntl(state->bootstrap.retained_parent_fd, F_DUPFD_CLOEXEC, 3);
  struct stat value{};
  if (state->parent < 0 || fstat(state->parent, &value) != 0 || !private_metadata(value, true) || !no_acl(state->parent)) return Status::Denied;
  state->parent_id = FileIdentity::From(value);
  if (!state->ParentValid()) return Status::Denied;
  *output = std::unique_ptr<OwnedIdentity>(new OwnedIdentity(std::move(state)));
  State& owner = *(*output)->state_;
  if (SecKeychainSetUserInteractionAllowed(false) != errSecSuccess || !owner.CaptureCode()) return Status::Denied;
  if (mkdirat(owner.parent, kDirectory, 0700) != 0) return Status::Denied;
  owner.created_directory = true;
  if (fstatat(owner.parent, kDirectory, &value, AT_SYMLINK_NOFOLLOW) != 0 || !private_metadata(value, true)) return Status::OutcomeUnknown;
  owner.directory_id = FileIdentity::From(value);
  owner.directory_path = owner.bootstrap.canonical_parent + "/" + kDirectory;
  owner.directory = openat(owner.parent, kDirectory, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
  if (!owner.NamespaceValid() || !owner.ExactEntries(false)) return Status::OutcomeUnknown;
  // SecKeychainCreate requires canonical UTF-8. Preserve 256-bit entropy while
  // encoding its password as printable ASCII, not unrestricted random bytes.
  std::array<std::uint8_t, 32> store_entropy{};
  std::array<std::uint8_t, 64> store_password{};
  WipeOnExit store_entropy_wipe{store_entropy};
  WipeOnExit store_password_wipe{store_password};
  if (SecRandomCopyBytes(kSecRandomDefault, store_entropy.size(), store_entropy.data()) != errSecSuccess) return Status::Unavailable;
  constexpr char password_hex[] = "0123456789abcdef";
  for (std::size_t index = 0; index < store_entropy.size(); ++index) {
    store_password[index * 2] = password_hex[store_entropy[index] >> 4];
    store_password[index * 2 + 1] = password_hex[store_entropy[index] & 15];
  }
  wipe(store_entropy);
  const std::string path = owner.directory_path + "/" + kStore;
  owner.creation_unknown = true;
  if (SecKeychainCreate(path.c_str(), store_password.size(), store_password.data(), false, nullptr, &owner.keychain) != errSecSuccess ||
      !owner.BindStore()) return Status::OutcomeUnknown;
  owner.creation_unknown = false;

  Ref<SecTrustedApplicationRef> broker;
  Ref<CFArrayRef> trusted;
  Ref<SecAccessRef> access;
  if (!owner.CodeValid() || SecTrustedApplicationCreateFromPath(nullptr, broker.out()) != errSecSuccess ||
      SecTrustedApplicationCopyData(broker.value, &owner.trusted_data) != errSecSuccess || !owner.trusted_data) return Status::Unavailable;
  const void* applications[] = {broker.value};
  trusted.value = CFArrayCreate(kCFAllocatorDefault, applications, 1, &kCFTypeArrayCallBacks);
  if (!trusted.value || SecAccessCreate(CFSTR("Colossus TLS broker identity"), trusted.value, access.out()) != errSecSuccess) return Status::Unavailable;
  Ref<CFArrayRef> acls;
  if (SecAccessCopyACLList(access.value, acls.out()) != errSecSuccess || !acls.value ||
      CFArrayGetCount(acls.value) == 0 || CFArrayGetCount(acls.value) > 32) return Status::Unavailable;
  // Every item ACL is limited to this broker app, including ACL mutation/export
  // authorizations. The imported key's separate usage is SIGN and nonextractable.
  for (CFIndex index = 0; index < CFArrayGetCount(acls.value); ++index) {
    SecACLRef acl = static_cast<SecACLRef>(const_cast<void*>(CFArrayGetValueAtIndex(acls.value, index)));
    if (SecACLSetContents(acl, trusted.value, CFSTR("Colossus TLS broker identity"), 0) != errSecSuccess) return Status::Unavailable;
  }
  const void* usages[] = {kSecAttrCanSign};
  const void* attributes[] = {kSecAttrIsPermanent, kSecAttrIsSensitive};
  Ref<CFArrayRef> key_usage;
  Ref<CFArrayRef> key_attributes;
  key_usage.value = CFArrayCreate(kCFAllocatorDefault, usages, 1, &kCFTypeArrayCallBacks);
  key_attributes.value = CFArrayCreate(kCFAllocatorDefault, attributes, 2, &kCFTypeArrayCallBacks);
  Ref<CFDataRef> encoded;
  Ref<CFDataRef> passphrase;
  encoded.value = CFDataCreateWithBytesNoCopy(kCFAllocatorDefault, enrollment.encrypted_pkcs12.data(),
      enrollment.encrypted_pkcs12.size(), kCFAllocatorNull);
  passphrase.value = CFDataCreateWithBytesNoCopy(kCFAllocatorDefault, enrollment.password.data(),
      enrollment.password.size(), kCFAllocatorNull);
  if (!key_usage.value || !key_attributes.value || !encoded.value || !passphrase.value || !owner.StoreValid() || !owner.CodeValid()) return Status::Unavailable;
  SecItemImportExportKeyParameters parameters{};
  parameters.version = SEC_KEY_IMPORT_EXPORT_PARAMS_VERSION;
  parameters.flags = kSecKeyImportOnlyOne;
  parameters.passphrase = passphrase.value;
  parameters.accessRef = access.value;
  parameters.keyUsage = key_usage.value;
  // NULL would default to extractable; this explicit array deliberately omits
  // kSecAttrIsExtractable. Never use kSecKeyNoAccessControl or UI passphrase flags.
  parameters.keyAttributes = key_attributes.value;
  SecExternalFormat format = kSecFormatPKCS12;
  SecExternalItemType type = kSecItemTypeAggregate;
  if (SecItemImport(encoded.value, CFSTR("p12"), &format, &type, 0, &parameters,
                    owner.keychain, &owner.imported) != errSecSuccess || !owner.imported ||
      format != kSecFormatPKCS12 || type != kSecItemTypeAggregate ||
      CFArrayGetCount(owner.imported) == 0 || CFArrayGetCount(owner.imported) > 16 || !owner.StoreValid()) return Status::OutcomeUnknown;
  for (CFIndex index = 0; index < CFArrayGetCount(owner.imported); ++index) {
    CFTypeRef item = CFArrayGetValueAtIndex(owner.imported, index);
    if (!item) return Status::Denied;
    if (CFGetTypeID(item) == SecIdentityGetTypeID()) {
      if (owner.identity) return Status::Denied;
      owner.identity = static_cast<SecIdentityRef>(const_cast<void*>(CFRetain(item)));
    } else if (CFGetTypeID(item) != SecCertificateGetTypeID()) return Status::Denied;
  }
  Ref<SecCertificateRef> certificate;
  Ref<CFDataRef> leaf;
  if (!owner.identity || SecIdentityCopyCertificate(owner.identity, certificate.out()) != errSecSuccess ||
      SecIdentityCopyPrivateKey(owner.identity, &owner.key) != errSecSuccess || !owner.key ||
      !owner.SameStore(reinterpret_cast<SecKeychainItemRef>(owner.key))) return Status::Denied;
  leaf.value = SecCertificateCopyData(certificate.value);
  if (!leaf.value || CFDataGetLength(leaf.value) != static_cast<CFIndex>(owner.leaf.size()) ||
      memcmp(CFDataGetBytePtr(leaf.value), owner.leaf.data(), owner.leaf.size()) ||
      !owner.bootstrap.validate.identity(owner.leaf) || !owner.KeyProperties() || !owner.AclValid() ||
      !owner.CodeValid() || !owner.StoreValid()) return Status::Denied;
  owner.ready = true;
  return Status::Ready;
}

Status OwnedIdentity::CopyPublicMaterial(const Generation& generation,
                                        std::vector<std::vector<std::uint8_t>>* ca_der,
                                        std::vector<std::uint8_t>* leaf_der) {
  std::lock_guard lock(state_->mutex);
  if (generation != state_->bootstrap.generation || !ca_der || !leaf_der ||
      !state_->Valid()) return Status::Denied;
  for (const auto& root : state_->roots) if (!state_->bootstrap.validate.ca(root)) return Status::Denied;
  *ca_der = state_->roots;
  *leaf_der = state_->leaf;
  return Status::Ready;
}

Status OwnedIdentity::BeginHandshake(const Generation& generation,
                                    const std::string& origin, const Fingerprint& leaf,
                                    std::chrono::steady_clock::time_point deadline, Handshake* output) {
  std::lock_guard lock(state_->mutex);
  State& owner = *state_;
  const auto now = std::chrono::steady_clock::now();
  if (owner.handshake && owner.handshake->second <= now) owner.handshake.reset();
  if (!output || owner.handshake || !owner.BindingValid(generation, origin, leaf) ||
      !owner.Valid() || deadline <= now || deadline - now > std::chrono::seconds(30) ||
      owner.sequence == UINT64_MAX) return Status::Denied;
  const auto sequence = ++owner.sequence;
  owner.handshake = std::pair{sequence, deadline};
  output->sequence = sequence;
  output->generation = generation;
  return Status::Ready;
}

Status OwnedIdentity::CertificateVerify(Handshake handshake, Tls13Scheme scheme,
                                      const std::array<std::uint8_t, 32>& transcript,
                                      std::vector<std::uint8_t>* signature) {
  std::lock_guard lock(state_->mutex);
  State& owner = *state_;
  if (signature) signature->clear();
  if (!signature || handshake.generation != owner.bootstrap.generation || !owner.handshake ||
      owner.handshake->first != handshake.sequence || !owner.Valid()) return Status::Denied;
  const auto deadline = owner.handshake->second;
  owner.handshake.reset();  // Consume before any signature/error; never retry it.
  if (std::chrono::steady_clock::now() >= deadline) return Status::Denied;
  Ref<CFDictionaryRef> attributes;
  attributes.value = SecKeyCopyAttributes(owner.key);
  if (!attributes.value) return Status::Unavailable;
  CFTypeRef type = CFDictionaryGetValue(attributes.value, kSecAttrKeyType);
  CFTypeRef size_value = CFDictionaryGetValue(attributes.value, kSecAttrKeySizeInBits);
  int size = 0;
  if (!type || !size_value || CFGetTypeID(size_value) != CFNumberGetTypeID() ||
      !CFNumberGetValue(static_cast<CFNumberRef>(size_value), kCFNumberIntType, &size)) return Status::Denied;
  SecKeyAlgorithm algorithm = nullptr;
  if (scheme == Tls13Scheme::EcdsaP256Sha256 && CFEqual(type, kSecAttrKeyTypeECSECPrimeRandom) && size == 256)
    algorithm = kSecKeyAlgorithmECDSASignatureMessageX962SHA256;
  else if (scheme == Tls13Scheme::RsaPssSha256 && CFEqual(type, kSecAttrKeyTypeRSA) && size >= 2048 && size <= 4096)
    algorithm = kSecKeyAlgorithmRSASignatureMessagePSSSHA256;
  if (!algorithm || !SecKeyIsAlgorithmSupported(owner.key, kSecKeyOperationTypeSign, algorithm)) return Status::Unavailable;
  constexpr char context[] = "TLS 1.3, client CertificateVerify";
  std::array<std::uint8_t, 64 + sizeof(context) + 32> message{};
  std::fill(message.begin(), message.begin() + 64, 0x20);
  memcpy(message.data() + 64, context, sizeof(context));  // includes fixed zero separator
  memcpy(message.data() + 64 + sizeof(context), transcript.data(), transcript.size());
  Ref<CFDataRef> data;
  Ref<CFDataRef> result;
  Ref<CFErrorRef> error;
  data.value = CFDataCreateWithBytesNoCopy(kCFAllocatorDefault, message.data(), message.size(), kCFAllocatorNull);
  if (!data.value) return Status::Unavailable;
  result.value = SecKeyCreateSignature(owner.key, algorithm, data.value, error.out());
  if (!result.value || CFDataGetLength(result.value) <= 0 || CFDataGetLength(result.value) > 512) return Status::Unavailable;
  if (!owner.Valid() || std::chrono::steady_clock::now() >= deadline) return Status::Denied;
  const auto* bytes = CFDataGetBytePtr(result.value);
  signature->assign(bytes, bytes + CFDataGetLength(result.value));
  return Status::Ready;
}

void OwnedIdentity::Revoke() {
  std::lock_guard lock(state_->mutex);
  state_->ready = false;
  state_->handshake.reset();
  state_->RetireGeneration();
}

void OwnedIdentity::CancelHandshake(Handshake handshake) {
  std::lock_guard lock(state_->mutex);
  if (handshake.generation == state_->bootstrap.generation && state_->handshake &&
      state_->handshake->first == handshake.sequence) state_->handshake.reset();
}

Status OwnedIdentity::Finish() {
  std::lock_guard lock(state_->mutex);
  State& owner = *state_;
  owner.ready = false;
  owner.handshake.reset();
  owner.RetireGeneration();
  using Cleanup = State::Cleanup;
  if (owner.cleanup == Cleanup::Finished) return Status::Ready;
  if (owner.creation_unknown || owner.changed) return Status::OutcomeUnknown;
  if (owner.directory < 0) {
    if (owner.created_directory) return Status::OutcomeUnknown;
    owner.cleanup = Cleanup::Finished;
    return Status::Ready;
  }
  if (owner.cleanup < Cleanup::DirectoryRemoved && !owner.NamespaceValid()) return Status::OutcomeUnknown;
  if (owner.cleanup == Cleanup::Live) {
    if (owner.store_recorded) {
      if (!owner.keychain || !owner.FileBound() || !owner.ExactEntries(true) ||
          !keychain_path(owner.keychain, owner.store_path) || SecKeychainLock(owner.keychain) != errSecSuccess ||
          !owner.FileBound()) return Status::OutcomeUnknown;
    } else if (owner.keychain || owner.imported || owner.identity || owner.key || owner.file >= 0 || !owner.ExactEntries(false)) return Status::OutcomeUnknown;
    owner.cleanup = Cleanup::Locked;
  }
  if (owner.cleanup == Cleanup::Locked) {
    if (!owner.NamespaceValid() || (owner.store_recorded && !owner.FileBound())) return Status::OutcomeUnknown;
    owner.ReleaseItems();
    if (owner.keychain) { CFRelease(owner.keychain); owner.keychain = nullptr; }
    owner.cleanup = Cleanup::Released;
  }
  if (owner.cleanup == Cleanup::Released) {
    if (!owner.NamespaceValid() || !owner.ExactEntries(owner.store_recorded) ||
        (owner.store_recorded && !owner.FileBound())) return Status::OutcomeUnknown;
    if (owner.retired_name.empty()) {
      std::array<std::uint8_t, 16> nonce{};
      if (SecRandomCopyBytes(kSecRandomDefault, nonce.size(), nonce.data()) != errSecSuccess) return Status::OutcomeUnknown;
      constexpr char hex[] = "0123456789abcdef";
      owner.retired_name = ".retired-tls-identity-";
      for (auto byte : nonce) { owner.retired_name += hex[byte >> 4]; owner.retired_name += hex[byte & 15]; }
    }
    if (renameatx_np(owner.parent, kDirectory, owner.parent, owner.retired_name.c_str(), RENAME_EXCL) != 0) return Status::OutcomeUnknown;
    owner.cleanup = Cleanup::Retired;
    if (!owner.NamespaceValid()) { owner.changed = true; return Status::OutcomeUnknown; }
  }
  if (owner.cleanup == Cleanup::Retired) {
    if (!owner.NamespaceValid() || !owner.ExactEntries(owner.store_recorded) ||
        (owner.store_recorded && !owner.FileBound())) return Status::OutcomeUnknown;
    if (owner.store_recorded && unlinkat(owner.directory, owner.store_name.c_str(), 0) != 0) return Status::OutcomeUnknown;
    owner.cleanup = Cleanup::FileRemoved;
  }
  if (owner.cleanup == Cleanup::FileRemoved) {
    if (!owner.NamespaceValid() || !owner.ExactEntries(false)) return Status::OutcomeUnknown;
    if (owner.file >= 0) { close(owner.file); owner.file = -1; }
    if (unlinkat(owner.parent, owner.retired_name.c_str(), AT_REMOVEDIR) != 0) return Status::OutcomeUnknown;
    owner.cleanup = Cleanup::DirectoryRemoved;
  }
  if (owner.cleanup == Cleanup::DirectoryRemoved) {
    struct stat held{}, entry{};
    if (!owner.ParentValid() || fstat(owner.directory, &held) != 0 || !owner.directory_id.Same(held) ||
        !private_metadata(held, true) || !no_acl(owner.directory) ||
        fstatat(owner.parent, owner.retired_name.c_str(), &entry, AT_SYMLINK_NOFOLLOW) == 0 || errno != ENOENT ||
        fsync(owner.parent) != 0) return Status::OutcomeUnknown;
    close(owner.directory); owner.directory = -1;
    close(owner.parent); owner.parent = -1;
    owner.cleanup = Cleanup::Finished;
  }
  return owner.cleanup == Cleanup::Finished ? Status::Ready : Status::OutcomeUnknown;
}
}  // namespace colossus::browser::pki::mac
#pragma clang diagnostic pop
