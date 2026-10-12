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
#include <cstdio>

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
  enum class Cleanup { Live, Locked, Released, Retired, FileRemoved, AuxiliaryRemoved, DirectoryRemoved, Finished };
  std::mutex mutex;
  Bootstrap bootstrap;
  std::vector<std::vector<std::uint8_t>> roots;
  std::vector<std::uint8_t> leaf;
  Fingerprint leaf_hash{};
  bool ready = false, changed = false, created_directory = false;
  bool creation_unknown = false, store_recorded = false;
  Cleanup cleanup = Cleanup::Live;
  int parent = -1, directory = -1, file = -1, auxiliary = -1;
  FileIdentity parent_id{}, directory_id{}, file_id{}, auxiliary_id{};
  std::string directory_path, store_path, store_name, retired_name, auxiliary_name;
  bool auxiliary_recorded = false;
  std::uint64_t mutation_sequence = 0;
  Generation custody_generation{};
  SecKeychainRef keychain = nullptr;
  CFArrayRef imported = nullptr;
  SecIdentityRef identity = nullptr;
  SecKeyRef key = nullptr;
  SecCodeRef code = nullptr;
  SecRequirementRef requirement = nullptr;
  CFDataRef trusted_data = nullptr;
  std::uint64_t sequence = 0;
  std::optional<std::pair<std::uint64_t, std::chrono::steady_clock::time_point>> handshake;
  std::vector<std::pair<SecKeychainItemRef, CFDataRef>> item_receipts;
  Diagnostic diagnostic;
  void Phase(DiagnosticPhase phase) { diagnostic = {phase, DiagnosticCheck::Start, 0}; }
  void Check(DiagnosticCheck check, OSStatus status = errSecSuccess) {
    diagnostic.check = check; diagnostic.os_status = status;
  }

  ~State() {
    // No destructor/atexit store mutation. Unknown physical state is retained
    // for the supervising owner; an early broker drop cannot certify cleanup.
    RetireGeneration();
    wipe(custody_generation);
    ReleaseItems();
    if (keychain) CFRelease(keychain);
    if (code) CFRelease(code);
    if (requirement) CFRelease(requirement);
    if (trusted_data) CFRelease(trusted_data);
    for (int descriptor : {file, auxiliary, directory, parent}) if (descriptor >= 0) close(descriptor);
  }
  void ReleaseItems() {
    for (auto& [item, receipt] : item_receipts) { CFRelease(item); CFRelease(receipt); }
    item_receipts.clear();
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
    const bool expect_auxiliary = auxiliary_recorded && cleanup < Cleanup::AuxiliaryRemoved;
    unsigned int count = 0, seen = 0;
    bool valid = true;
    while (true) {
      errno = 0;
      dirent* entry = readdir(entries);
      if (!entry) { if (errno) valid = false; break; }
      const std::string name = entry->d_name;
      if (name == "." || name == "..") continue;
      struct stat value{};
      const bool store = expect_store && name == store_name;
      const bool aux = expect_auxiliary && name == auxiliary_name;
      const unsigned int bit = store ? 1 : aux ? 2 : 0;
      if (!bit || (seen & bit) || ++count > 2 ||
          fstatat(directory, name.c_str(), &value, AT_SYMLINK_NOFOLLOW) != 0 ||
          !(store ? file_id : auxiliary_id).Same(value) || !private_metadata(value, false) ||
          (aux && value.st_size != 0)) { valid = false; break; }
      seen |= bit;
    }
    closedir(entries);
    return valid && count == unsigned(expect_store) + unsigned(expect_auxiliary);
  }
  bool AuxiliaryBound() const {
    struct stat held{}, entry{};
    return auxiliary_recorded && auxiliary >= 0 && NamespaceValid() &&
        fstat(auxiliary, &held) == 0 && auxiliary_id.Same(held) &&
        private_metadata(held, false) && held.st_size == 0 && no_acl(auxiliary) &&
        fstatat(directory, auxiliary_name.c_str(), &entry, AT_SYMLINK_NOFOLLOW) == 0 &&
        auxiliary_id.Same(entry) && private_metadata(entry, false) && entry.st_size == 0;
  }
  bool StoreBound() const {
    return cleanup == Cleanup::Live && FileBound() &&
        (!auxiliary_recorded || AuxiliaryBound()) && ExactEntries(true) &&
        keychain_path(keychain, store_path);
  }
  bool StoreValid() {
    SecKeychainStatus status = 0;
    Check(DiagnosticCheck::Binding);
    if (!StoreBound()) return false;
    const OSStatus observed = SecKeychainGetStatus(keychain, &status);
    Check(DiagnosticCheck::StoreStatus, observed);
    if (observed != errSecSuccess) return false;
    Check(DiagnosticCheck::StoreLocked);
    return status & kSecUnlockStateStatus;
  }
  bool CaptureGeneration(MutationWindow& window, bool initial) {
    char path[PATH_MAX];
    UInt32 length = sizeof(path);
    Check(DiagnosticCheck::Path);
    if (!keychain) return false;
    const OSStatus observed = SecKeychainGetPath(keychain, &length, path);
    Check(DiagnosticCheck::Path, observed);
    if (observed != errSecSuccess || length >= sizeof(path)) return false;
    const std::string returned(path, length);
    if (!CaptureOutputs(window, initial, returned)) return false;
    // A successful native operation plus independently sealed exact outputs
    // proves physical ownership even when later readiness validation fails.
    // Retirement uses this positive generation; failed/unsealed operations keep
    // creation_unknown and may never be adopted from a later pathname snapshot.
    creation_unknown = false;
    return StoreValid();
  }
  bool CaptureOutputs(MutationWindow& window, bool initial, const std::string& returned) {
    const std::string expected = directory_path + "/" + kStore;
    Check(DiagnosticCheck::Path);
    if (!window.Exclusive() || !NamespaceValid() ||
        (initial ? returned != expected && returned != expected + "-db" : returned != store_path)) return false;
    const std::string name = returned.substr(directory_path.size() + 1);
    std::array<std::uint8_t, CC_SHA1_DIGEST_LENGTH> digest{};
    if (!CC_SHA1(name.data(), name.size(), digest.data())) return false;
    char lock_name[12];
    snprintf(lock_name, sizeof(lock_name), ".fl%02X%02X%02X%02X", digest[0], digest[1], digest[2], digest[3]);
    const int new_file = openat(directory, name.c_str(), O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
    const int new_aux = openat(directory, lock_name, O_RDONLY | O_NOFOLLOW | O_CLOEXEC);
    struct stat value{}, aux_value{};
    const auto native_output = [](int fd, struct stat* output) {
      return fd >= 0 && fstat(fd, output) == 0 && output->st_uid == geteuid() &&
          S_ISREG(output->st_mode) && output->st_nlink == 1 && no_acl(fd);
    };
    Check(DiagnosticCheck::Metadata);
    bool valid = native_output(new_file, &value) && native_output(new_aux, &aux_value) && aux_value.st_size == 0;
    if (valid) {
      Check(DiagnosticCheck::Entries);
      const int copy = fcntl(directory, F_DUPFD_CLOEXEC, 3);
      DIR* entries = copy >= 0 ? fdopendir(copy) : nullptr;
      if (!entries) { if (copy >= 0) close(copy); valid = false; }
      else {
        rewinddir(entries);
        unsigned int seen = 0;
        while (valid) {
          errno = 0;
          const dirent* entry = readdir(entries);
          if (!entry) { if (errno) valid = false; break; }
          const std::string current = entry->d_name;
          if (current == "." || current == "..") continue;
          const unsigned int bit = current == name ? 1 : current == lock_name ? 2 : 0;
          struct stat metadata{};
          if (!bit || (seen & bit) || fstatat(directory, current.c_str(), &metadata, AT_SYMLINK_NOFOLLOW) != 0 ||
              !FileIdentity::From(bit == 1 ? value : aux_value).Same(metadata)) valid = false;
          seen |= bit;
        }
        if (seen != 3) valid = false;
        closedir(entries);
      }
    }
    // Hardening is authorized only within the independently exclusive positive
    // native-output window. No post-failure stat or basename grants ownership.
    const auto harden = [](int fd, struct stat* output) {
      return fd >= 0 && fstat(fd, output) == 0 && output->st_uid == geteuid() &&
          S_ISREG(output->st_mode) && output->st_nlink == 1 && no_acl(fd) &&
          fchmod(fd, 0600) == 0 && fstat(fd, output) == 0 && private_metadata(*output, false);
    };
    if (!valid) { if (new_file >= 0) close(new_file); if (new_aux >= 0) close(new_aux); return false; }
    Check(DiagnosticCheck::Harden);
    valid = (initial || (auxiliary_recorded && auxiliary_id.Same(aux_value))) &&
        window.Exclusive() && harden(new_file, &value) && harden(new_aux, &aux_value) &&
        aux_value.st_size == 0 && window.Exclusive();
    const std::array<int, 2> outputs{new_file, new_aux};
    if (valid) { Check(DiagnosticCheck::Seal); valid = window.Seal(mutation_sequence, outputs) && window.Exclusive(); }
    if (!valid) { if (new_file >= 0) close(new_file); if (new_aux >= 0) close(new_aux); return false; }
    if (file >= 0) close(file);
    if (auxiliary >= 0) close(auxiliary);
    file = new_file; auxiliary = new_aux;
    file_id = FileIdentity::From(value); auxiliary_id = FileIdentity::From(aux_value);
    store_path = returned; store_name = name; auxiliary_name = lock_name;
    store_recorded = true; auxiliary_recorded = true;
    Check(DiagnosticCheck::Binding);
    return FileBound() && AuxiliaryBound() && ExactEntries(true);
  }
  std::unique_ptr<MutationWindow> BeginMutation(Mutation operation) {
    if (!bootstrap.mutations || mutation_sequence == UINT64_MAX || !ParentValid()) return {};
    if (operation == Mutation::CreateNamespace) {
      if (directory >= 0 || created_directory) return {};
    } else if (operation == Mutation::RetireStore) {
      if (cleanup < Cleanup::DirectoryRemoved && !NamespaceValid()) return {};
    } else if (!NamespaceValid() ||
               (operation == Mutation::CreateStore ? !ExactEntries(false) : !StoreBound())) return {};
    auto window = bootstrap.mutations->Begin(custody_generation, parent, directory, ++mutation_sequence, operation);
    if (!window || !window->Exclusive()) return {};
    return window;
  }
  Status CreateNamespace() {
    Phase(DiagnosticPhase::Namespace);
    Check(DiagnosticCheck::Window);
    auto window = BeginMutation(Mutation::CreateNamespace);
    if (!window) return Status::Denied;
    Check(DiagnosticCheck::NativeApi);
    if (mkdirat(parent, kDirectory, 0700) != 0) return Status::Denied;
    created_directory = true;
    creation_unknown = true;
    struct stat value{};
    Check(DiagnosticCheck::Metadata);
    if (fstatat(parent, kDirectory, &value, AT_SYMLINK_NOFOLLOW) != 0 || !private_metadata(value, true)) return Status::OutcomeUnknown;
    directory_id = FileIdentity::From(value);
    directory_path = bootstrap.canonical_parent + "/" + kDirectory;
    directory = openat(parent, kDirectory, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
    const std::array<int, 1> output{directory};
    Check(DiagnosticCheck::Seal);
    if (!NamespaceValid() || !ExactEntries(false) || !window->Exclusive() ||
        !window->Seal(mutation_sequence, output) || !window->Exclusive()) return Status::OutcomeUnknown;
    window.reset();
    creation_unknown = false;
    return Status::Ready;
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
  bool RecordItem(SecKeychainItemRef item) {
    Ref<CFDataRef> receipt;
    if (!SameStore(item) || SecKeychainItemCreatePersistentReference(item, receipt.out()) != errSecSuccess ||
        !receipt.value || CFDataGetLength(receipt.value) <= 0 || CFDataGetLength(receipt.value) > 65536) return false;
    for (const auto& existing : item_receipts) if (CFEqual(existing.second, receipt.value)) return true;
    if (item_receipts.size() >= 17) return false;
    CFRetain(item);
    item_receipts.emplace_back(item, receipt.release());
    return true;
  }
  bool ItemsValid() const {
    if (item_receipts.empty() || item_receipts.size() > 17) return false;
    for (const auto& [item, expected] : item_receipts) {
      Ref<CFDataRef> actual;
      if (!SameStore(item) || SecKeychainItemCreatePersistentReference(item, actual.out()) != errSecSuccess ||
          !actual.value || !CFEqual(expected, actual.value)) return false;
    }
    std::size_t count = 0;
    for (SecItemClass kind : {kSecPrivateKeyItemClass, kSecCertificateItemClass,
                             kSecPublicKeyItemClass, kSecSymmetricKeyItemClass,
                             kSecGenericPasswordItemClass, kSecInternetPasswordItemClass}) {
      Ref<SecKeychainSearchRef> search;
      if (SecKeychainSearchCreateFromAttributes(keychain, kind, nullptr, search.out()) != errSecSuccess || !search.value) return false;
      while (true) {
        Ref<SecKeychainItemRef> item;
        const auto status = SecKeychainSearchCopyNext(search.value, item.out());
        if (status == errSecItemNotFound) break;
        if (status != errSecSuccess || !item.value || ++count > item_receipts.size()) return false;
        Ref<CFDataRef> receipt;
        if (!SameStore(item.value) || SecKeychainItemCreatePersistentReference(item.value, receipt.out()) != errSecSuccess ||
            !receipt.value || std::none_of(item_receipts.begin(), item_receipts.end(),
                [&receipt](const auto& expected) { return CFEqual(expected.second, receipt.value); })) return false;
      }
    }
    return count == item_receipts.size();
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
  bool SigningAdmitted() {
    if (bootstrap.mutations && bootstrap.mutations->SigningAdmitted(bootstrap.generation)) return true;
    ready = false;
    handshake.reset();
    RetireGeneration();
    return false;
  }
  bool Valid() {
    if (!ready || changed || cleanup != Cleanup::Live) return false;
    if (!SigningAdmitted()) return false;
    if (!CodeValid() || !StoreBound() || !SameStore(reinterpret_cast<SecKeychainItemRef>(key)) ||
        !KeyProperties() || !AclValid() || !ItemsValid()) {
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
      !bootstrap.mutations || !bootstrap.mutations->SigningAdmitted(bootstrap.generation) ||
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
  state->custody_generation = state->bootstrap.generation;
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
  const OSStatus interaction = SecKeychainSetUserInteractionAllowed(false);
  owner.Check(DiagnosticCheck::NativeApi, interaction);
  if (interaction != errSecSuccess) return Status::Denied;
  owner.Check(DiagnosticCheck::Code);
  if (!owner.CaptureCode()) return Status::Denied;
  const auto namespace_status = owner.CreateNamespace();
  if (namespace_status != Status::Ready) return namespace_status;
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
  owner.Phase(DiagnosticPhase::Create);
  owner.Check(DiagnosticCheck::Window);
  auto creation = owner.BeginMutation(Mutation::CreateStore);
  if (!creation) return Status::Denied;
  owner.creation_unknown = true;
  const OSStatus created = SecKeychainCreate(path.c_str(), store_password.size(), store_password.data(), false, nullptr, &owner.keychain);
  owner.Check(DiagnosticCheck::NativeApi, created);
  if (created != errSecSuccess) return Status::OutcomeUnknown;
  owner.Check(DiagnosticCheck::Binding);
  if (!creation->Exclusive() || !owner.keychain ||
      !(keychain_path(owner.keychain,path)||keychain_path(owner.keychain,path+"-db"))) return Status::OutcomeUnknown;
  // Use only the fresh store's generated password and explicit native reference;
  // no default-keychain lookup, passphrase prompt or global preferences change.
  // Creation/unlock share the same independent writer-exclusion window, and
  // their exact output generation is sealed once after both positive results.
  const OSStatus unlocked = SecKeychainUnlock(owner.keychain, store_password.size(), store_password.data(), true);
  owner.Check(DiagnosticCheck::Unlock, unlocked);
  if (unlocked != errSecSuccess || !creation->Exclusive() || !owner.CaptureGeneration(*creation, true)) return Status::OutcomeUnknown;
  creation.reset();
  owner.creation_unknown = false;

  Ref<SecTrustedApplicationRef> broker;
  Ref<CFArrayRef> trusted;
  Ref<SecAccessRef> access;
  owner.Check(DiagnosticCheck::Access);
  if (!owner.CodeValid()) return Status::Denied;
  const OSStatus application = SecTrustedApplicationCreateFromPath(nullptr, broker.out());
  owner.Check(DiagnosticCheck::Access, application);
  if (application != errSecSuccess) return Status::Unavailable;
  const OSStatus trusted_bytes = SecTrustedApplicationCopyData(broker.value, &owner.trusted_data);
  owner.Check(DiagnosticCheck::Access, trusted_bytes);
  if (trusted_bytes != errSecSuccess || !owner.trusted_data) return Status::Unavailable;
  const void* applications[] = {broker.value};
  trusted.value = CFArrayCreate(kCFAllocatorDefault, applications, 1, &kCFTypeArrayCallBacks);
  if (!trusted.value) return Status::Unavailable;
  const OSStatus created_access = SecAccessCreate(CFSTR("Colossus TLS broker identity"), trusted.value, access.out());
  owner.Check(DiagnosticCheck::Access, created_access);
  if (created_access != errSecSuccess) return Status::Unavailable;
  Ref<CFArrayRef> acls;
  const OSStatus copied_acl = SecAccessCopyACLList(access.value, acls.out());
  owner.Check(DiagnosticCheck::Acl, copied_acl);
  if (copied_acl != errSecSuccess || !acls.value ||
      CFArrayGetCount(acls.value) == 0 || CFArrayGetCount(acls.value) > 32) return Status::Unavailable;
  // Every item ACL is limited to this broker app, including ACL mutation/export
  // authorizations. The imported key's separate usage is SIGN and nonextractable.
  for (CFIndex index = 0; index < CFArrayGetCount(acls.value); ++index) {
    SecACLRef acl = static_cast<SecACLRef>(const_cast<void*>(CFArrayGetValueAtIndex(acls.value, index)));
    const OSStatus set_acl = SecACLSetContents(acl, trusted.value, CFSTR("Colossus TLS broker identity"), 0);
    owner.Check(DiagnosticCheck::Acl, set_acl);
    if (set_acl != errSecSuccess) return Status::Unavailable;
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
  owner.Phase(DiagnosticPhase::Import);
  owner.Check(DiagnosticCheck::Window);
  auto importing = owner.BeginMutation(Mutation::ImportIdentity);
  if (!importing) return Status::Denied;
  owner.creation_unknown = true;
  const OSStatus imported = SecItemImport(encoded.value, CFSTR("p12"), &format, &type, 0, &parameters,
                    owner.keychain, &owner.imported);
  owner.Check(DiagnosticCheck::NativeApi, imported);
  if (imported != errSecSuccess) {
    // Unknown imported contents do not erase previously proved physical
    // ownership. Only the SAME retained pre-import store/lock generations may
    // be quarantined for retirement; no failed output is captured or adopted.
    const std::array<int,2> previous{owner.file,owner.auxiliary};
    if (owner.StoreBound() && importing->RetireUnchanged(owner.mutation_sequence,previous))
      owner.creation_unknown=false;
    owner.ready=false;owner.handshake.reset();owner.RetireGeneration();
    return Status::OutcomeUnknown;
  }
  if (!owner.CaptureGeneration(*importing, false)) return Status::OutcomeUnknown;
  owner.Check(DiagnosticCheck::ImportShape);
  if (!owner.imported || format != kSecFormatPKCS12 || type != kSecItemTypeAggregate ||
      CFArrayGetCount(owner.imported) == 0 || CFArrayGetCount(owner.imported) > 16) return Status::Denied;
  for (CFIndex index = 0; index < CFArrayGetCount(owner.imported); ++index) {
    CFTypeRef item = CFArrayGetValueAtIndex(owner.imported, index);
    if (!item) return Status::Denied;
    if (CFGetTypeID(item) == SecIdentityGetTypeID()) {
      if (owner.identity) return Status::Denied;
      owner.identity = static_cast<SecIdentityRef>(const_cast<void*>(CFRetain(item)));
    } else if (CFGetTypeID(item) != SecCertificateGetTypeID() ||
               !owner.RecordItem(reinterpret_cast<SecKeychainItemRef>(const_cast<void*>(item)))) return Status::Denied;
  }
  Ref<SecCertificateRef> certificate;
  Ref<CFDataRef> leaf;
  owner.Phase(DiagnosticPhase::Validate);
  owner.Check(DiagnosticCheck::Leaf);
  if (!owner.identity || SecIdentityCopyCertificate(owner.identity, certificate.out()) != errSecSuccess ||
      SecIdentityCopyPrivateKey(owner.identity, &owner.key) != errSecSuccess || !owner.key ||
      !owner.SameStore(reinterpret_cast<SecKeychainItemRef>(owner.key)) ||
      !owner.RecordItem(reinterpret_cast<SecKeychainItemRef>(owner.key)) ||
      !owner.RecordItem(reinterpret_cast<SecKeychainItemRef>(certificate.value))) return Status::Denied;
  leaf.value = SecCertificateCopyData(certificate.value);
  if (!leaf.value || CFDataGetLength(leaf.value) != static_cast<CFIndex>(owner.leaf.size()) ||
      memcmp(CFDataGetBytePtr(leaf.value), owner.leaf.data(), owner.leaf.size()) ||
      !owner.bootstrap.validate.identity(owner.leaf)) return Status::Denied;
  owner.Check(DiagnosticCheck::Key); if (!owner.KeyProperties()) return Status::Denied;
  owner.Check(DiagnosticCheck::Acl); if (!owner.AclValid()) return Status::Denied;
  owner.Check(DiagnosticCheck::Code); if (!owner.CodeValid()) return Status::Denied;
  if (!owner.StoreValid()) return Status::Denied;
  owner.Check(DiagnosticCheck::Items); if (!owner.ItemsValid()) return Status::Denied;
  owner.Check(DiagnosticCheck::Window); if (!importing->Exclusive()) return Status::Denied;
  importing.reset();
  owner.creation_unknown = false;
  owner.ready = true;
  if (!owner.SigningAdmitted()) return Status::Denied;
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
  if (!owner.SigningAdmitted()) return Status::Denied;
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
  if (!owner.SigningAdmitted()) return Status::Denied;
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
  owner.Phase(DiagnosticPhase::Retire);
  owner.ready = false;
  owner.handshake.reset();
  owner.RetireGeneration();
  using Cleanup = State::Cleanup;
  if (owner.cleanup == Cleanup::Finished) return Status::Ready;
  owner.Check(DiagnosticCheck::Binding);
  if (owner.creation_unknown || owner.changed) return Status::OutcomeUnknown;
  if (owner.directory < 0) {
    if (owner.created_directory) return Status::OutcomeUnknown;
    owner.cleanup = Cleanup::Finished;
    wipe(owner.custody_generation);
    return Status::Ready;
  }
  owner.Check(DiagnosticCheck::Window);
  auto retiring = owner.BeginMutation(Mutation::RetireStore);
  if (!retiring || !retiring->Exclusive()) return Status::OutcomeUnknown;
  if (owner.cleanup < Cleanup::DirectoryRemoved && !owner.NamespaceValid()) return Status::OutcomeUnknown;
  if (owner.cleanup == Cleanup::Live) {
    owner.Check(DiagnosticCheck::Binding);
    if (!retiring->Exclusive()) return Status::OutcomeUnknown;
    if (owner.store_recorded) {
      if (!owner.keychain || !owner.FileBound() || !owner.ExactEntries(true) ||
          (owner.auxiliary_recorded && !owner.AuxiliaryBound()) ||
          !keychain_path(owner.keychain, owner.store_path)) return Status::OutcomeUnknown;
      const OSStatus locked = SecKeychainLock(owner.keychain);
      owner.Check(DiagnosticCheck::Lock, locked);
      if (locked != errSecSuccess || !owner.FileBound()) return Status::OutcomeUnknown;
    } else if (owner.keychain || owner.imported || owner.identity || owner.key || owner.file >= 0 || !owner.ExactEntries(false)) return Status::OutcomeUnknown;
    owner.cleanup = Cleanup::Locked;
  }
  if (owner.cleanup == Cleanup::Locked) {
    if (!retiring->Exclusive()) return Status::OutcomeUnknown;
    if (!owner.NamespaceValid() || (owner.store_recorded && !owner.FileBound()) ||
        (owner.auxiliary_recorded && !owner.AuxiliaryBound())) return Status::OutcomeUnknown;
    owner.ReleaseItems();
    if (owner.keychain) { CFRelease(owner.keychain); owner.keychain = nullptr; }
    owner.cleanup = Cleanup::Released;
  }
  if (owner.cleanup == Cleanup::Released) {
    if (!retiring->Exclusive()) return Status::OutcomeUnknown;
    if (!owner.NamespaceValid() || !owner.ExactEntries(owner.store_recorded) ||
        (owner.store_recorded && !owner.FileBound()) ||
        (owner.auxiliary_recorded && !owner.AuxiliaryBound())) return Status::OutcomeUnknown;
    if (owner.retired_name.empty()) {
      std::array<std::uint8_t, 16> nonce{};
      if (SecRandomCopyBytes(kSecRandomDefault, nonce.size(), nonce.data()) != errSecSuccess) return Status::OutcomeUnknown;
      constexpr char hex[] = "0123456789abcdef";
      owner.retired_name = ".retired-tls-identity-";
      for (auto byte : nonce) { owner.retired_name += hex[byte >> 4]; owner.retired_name += hex[byte & 15]; }
    }
    owner.Check(DiagnosticCheck::Rename);
    if (renameatx_np(owner.parent, kDirectory, owner.parent, owner.retired_name.c_str(), RENAME_EXCL) != 0) return Status::OutcomeUnknown;
    owner.cleanup = Cleanup::Retired;
    if (!owner.NamespaceValid()) { owner.changed = true; return Status::OutcomeUnknown; }
  }
  if (owner.cleanup == Cleanup::Retired) {
    owner.Check(DiagnosticCheck::Seal);
    if (!retiring->Checkpoint(owner.mutation_sequence, 0)) return Status::OutcomeUnknown;
    if (!retiring->Exclusive()) return Status::OutcomeUnknown;
    if (!owner.NamespaceValid() || !owner.ExactEntries(owner.store_recorded) ||
        (owner.store_recorded && !owner.FileBound()) ||
        (owner.auxiliary_recorded && !owner.AuxiliaryBound())) return Status::OutcomeUnknown;
    owner.Check(DiagnosticCheck::RemoveStore);
    if (owner.store_recorded && unlinkat(owner.directory, owner.store_name.c_str(), 0) != 0) return Status::OutcomeUnknown;
    owner.cleanup = Cleanup::FileRemoved;
  }
  if (owner.cleanup == Cleanup::FileRemoved) {
    owner.Check(DiagnosticCheck::Seal);
    if (!retiring->Checkpoint(owner.mutation_sequence, 1)) return Status::OutcomeUnknown;
    if (!retiring->Exclusive()) return Status::OutcomeUnknown;
    if (!owner.NamespaceValid() || !owner.ExactEntries(false) ||
        (owner.auxiliary_recorded && !owner.AuxiliaryBound())) return Status::OutcomeUnknown;
    owner.Check(DiagnosticCheck::RemoveAuxiliary);
    if (owner.auxiliary_recorded && unlinkat(owner.directory, owner.auxiliary_name.c_str(), 0) != 0) return Status::OutcomeUnknown;
    owner.cleanup = Cleanup::AuxiliaryRemoved;
  }
  if (owner.cleanup == Cleanup::AuxiliaryRemoved) {
    owner.Check(DiagnosticCheck::Seal);
    if (!retiring->Checkpoint(owner.mutation_sequence, 2)) return Status::OutcomeUnknown;
    if (!retiring->Exclusive()) return Status::OutcomeUnknown;
    if (!owner.NamespaceValid() || !owner.ExactEntries(false)) return Status::OutcomeUnknown;
    if (owner.file >= 0) { close(owner.file); owner.file = -1; }
    if (owner.auxiliary >= 0) { close(owner.auxiliary); owner.auxiliary = -1; }
    owner.Check(DiagnosticCheck::RemoveDirectory);
    if (unlinkat(owner.parent, owner.retired_name.c_str(), AT_REMOVEDIR) != 0) return Status::OutcomeUnknown;
    owner.cleanup = Cleanup::DirectoryRemoved;
  }
  if (owner.cleanup == Cleanup::DirectoryRemoved) {
    owner.Check(DiagnosticCheck::Sync);
    if (!retiring->Exclusive()) return Status::OutcomeUnknown;
    struct stat held{}, entry{};
    if (!owner.ParentValid() || fstat(owner.directory, &held) != 0 || !owner.directory_id.Same(held) ||
        !private_metadata(held, true) || !no_acl(owner.directory) ||
        fstatat(owner.parent, owner.retired_name.c_str(), &entry, AT_SYMLINK_NOFOLLOW) == 0 || errno != ENOENT ||
        fsync(owner.parent) != 0 || !retiring->Exclusive()) return Status::OutcomeUnknown;
    owner.Check(DiagnosticCheck::Seal);
    if (!retiring->Checkpoint(owner.mutation_sequence, 3)) return Status::OutcomeUnknown;
    // End the exclusion while every borrowed request descriptor remains live.
    // No filesystem mutation follows the final durable absence proof.
    retiring.reset();
    close(owner.directory); owner.directory = -1;
    close(owner.parent); owner.parent = -1;
    owner.cleanup = Cleanup::Finished;
    wipe(owner.custody_generation);
  }
  return owner.cleanup == Cleanup::Finished ? Status::Ready : Status::OutcomeUnknown;
}
Diagnostic OwnedIdentity::CopyDiagnostic() {
  std::lock_guard lock(state_->mutex);
  return state_->diagnostic;
}
}  // namespace colossus::browser::pki::mac
#pragma clang diagnostic pop
