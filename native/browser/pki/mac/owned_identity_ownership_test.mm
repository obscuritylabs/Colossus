// File-only regression of the real owner state machine. No Keychain, signing,
// code, item, audit-port, or browser APIs are invoked or references constructed.
#include <unistd.h>
#include <sys/wait.h>
#include <cassert>
#include <cerrno>
#include <cstdio>
#include <membership.h>
#include <uuid/uuid.h>

static bool fail_flush = false;
static int fixture_fsync(int descriptor) {
  if (fail_flush) { fail_flush = false; errno = EIO; return -1; }
  return fsync(descriptor);
}
#define fsync fixture_fsync
#define COLOSSUS_MAC_OWNED_PKI_FILE_ONLY_TEST 1
#include "owned_identity.mm"
#undef fsync

namespace colossus::browser::pki::mac {
struct FileOnlyFixture {
  struct CleanupWindow final : MutationWindow {
    int parent, directory;
    CleanupWindow(int parent_fd, int directory_fd) : parent(parent_fd), directory(directory_fd) {}
    ~CleanupWindow() { struct stat metadata{}; assert(fstat(parent, &metadata) == 0 && fstat(directory, &metadata) == 0); }
    bool Exclusive() const override { return true; }
    bool Seal(std::uint64_t, std::span<const int>) override { return false; }
    bool Checkpoint(std::uint64_t, std::uint32_t step) override { return step <= 3; }
    bool RetireUnchanged(std::uint64_t,std::span<const int>) override {return false;}
  };
  struct CleanupAuthority final : MutationAuthority {
    bool enabled = true;
    bool SigningAdmitted(const Generation&) const override { return false; }
    std::unique_ptr<MutationWindow> Begin(const Generation& generation, int parent, int directory,
        std::uint64_t sequence, Mutation operation) override {
      assert(generation == GenerationFilled() && parent >= 0 && directory >= 0 && sequence > 0 && operation == Mutation::RetireStore);
      return enabled ? std::make_unique<CleanupWindow>(parent, directory) : nullptr;
    }
    static Generation GenerationFilled() { Generation result{}; result.fill(0x42); return result; }
  };
  static bool ValidOrigin(const std::string& origin) {
    return origin == "https://allowed.example";
  }
  static std::unique_ptr<OwnedIdentity> Setup(bool record) {
    char path[] = "/private/tmp/colossus-owned-pki-files.XXXXXX";
    assert(mkdtemp(path));
    auto state = std::make_unique<OwnedIdentity::State>();
    state->bootstrap.canonical_parent = path;
    state->bootstrap.generation.fill(0x42);
    state->custody_generation = state->bootstrap.generation;
    state->bootstrap.mutations = std::make_shared<CleanupAuthority>();
    state->bootstrap.validate.origin = ValidOrigin;
    state->leaf_hash.fill(0x24);
    state->bootstrap.bindings.push_back(
        {"https://allowed.example", state->leaf_hash});
    state->parent = open(path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
    struct stat value{};
    assert(fstat(state->parent, &value) == 0);
    state->parent_id = FileIdentity::From(value);
    assert(mkdirat(state->parent, kDirectory, 0700) == 0);
    state->created_directory = true;
    state->directory_path = state->bootstrap.canonical_parent + "/" + kDirectory;
    state->directory = openat(state->parent, kDirectory, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC);
    assert(fstat(state->directory, &value) == 0);
    state->directory_id = FileIdentity::From(value);
    const int file = openat(state->directory, kStore, O_CREAT | O_EXCL | O_RDWR | O_NOFOLLOW | O_CLOEXEC, 0600);
    assert(file >= 0 && write(file, "fixture", 7) == 7);
    state->retired_name = ".retired-tls-identity-fixture";
    if (record) {
      state->file = file;
      state->store_recorded = true;
      state->store_name = kStore;
      state->store_path = state->directory_path + "/" + kStore;
      assert(fstat(file, &value) == 0);
      state->file_id = FileIdentity::From(value);
      // Skip all Security handle states: this fixture owns plain POSIX files.
      state->cleanup = OwnedIdentity::State::Cleanup::Released;
    } else close(file);
    assert(state->NamespaceValid());
    return std::unique_ptr<OwnedIdentity>(new OwnedIdentity(std::move(state)));
  }
  static void RemoveFixture(std::unique_ptr<OwnedIdentity> owner) {
    auto& state = *owner->state_;
    const std::string parent = state.bootstrap.canonical_parent;
    // Each of these exact plain files was created by this isolated fixture.
    for (const char* name : {kStore, "extra", "keep-original"}) unlinkat(state.directory, name, 0);
    if (!state.auxiliary_name.empty()) unlinkat(state.directory, state.auxiliary_name.c_str(), 0);
    unlinkat(state.directory, "keep-aux", 0);
    unlinkat(state.parent, kDirectory, AT_REMOVEDIR);
    unlinkat(state.parent, state.retired_name.c_str(), AT_REMOVEDIR);
    owner.reset();
    assert(rmdir(parent.c_str()) == 0);
  }
  static void UnknownCreation() {
    auto owner = Setup(false);
    owner->state_->creation_unknown = true;
    assert(owner->Finish() == Status::OutcomeUnknown);
    struct stat value{};
    assert(fstatat(owner->state_->directory, kStore, &value, AT_SYMLINK_NOFOLLOW) == 0);
    RemoveFixture(std::move(owner));
  }
  static void ExtraEntry() {
    auto owner = Setup(true);
    const int extra = openat(owner->state_->directory, "extra", O_CREAT | O_EXCL | O_WRONLY | O_CLOEXEC, 0600);
    assert(extra >= 0); close(extra);
    assert(owner->Finish() == Status::OutcomeUnknown);
    assert(owner->state_->cleanup == OwnedIdentity::State::Cleanup::Released);
    RemoveFixture(std::move(owner));
  }
  static void ReplacedStore() {
    auto owner = Setup(true);
    auto& state = *owner->state_;
    assert(renameat(state.directory, kStore, state.directory, "keep-original") == 0);
    const int replacement = openat(state.directory, kStore, O_CREAT | O_EXCL | O_WRONLY | O_CLOEXEC, 0600);
    assert(replacement >= 0); close(replacement);
    assert(owner->Finish() == Status::OutcomeUnknown);
    struct stat value{};
    assert(fstatat(state.directory, kStore, &value, AT_SYMLINK_NOFOLLOW) == 0 && !state.file_id.Same(value));
    assert(fstatat(state.directory, "keep-original", &value, AT_SYMLINK_NOFOLLOW) == 0 && state.file_id.Same(value));
    RemoveFixture(std::move(owner));
  }
  static void DerivedLockRejected() {
    auto owner = Setup(true);
    auto& state = *owner->state_;
    // This resembles the auxiliary file created by Apple's file-Keychain
    // provider. A recognized native basename is not an ownership receipt.
    const int lock = openat(state.directory, ".fl9F52894F",
                            O_CREAT | O_EXCL | O_WRONLY | O_CLOEXEC, 0444);
    assert(lock >= 0); close(lock);
    assert(!state.ExactEntries(true));
    assert(owner->Finish() == Status::OutcomeUnknown);
    assert(state.cleanup == OwnedIdentity::State::Cleanup::Released);
    assert(unlinkat(state.directory, ".fl9F52894F", 0) == 0);
    RemoveFixture(std::move(owner));
  }
  static void StoreAliasRejected() {
    auto owner = Setup(true);
    auto& state = *owner->state_;
    // An alias changes the retained store's link count and adds another deletion
    // target. The cleanup owner cannot infer who created it or remove either name.
    assert(linkat(state.directory, kStore, state.directory, "extra", 0) == 0);
    struct stat value{};
    assert(!state.FileBound());
    assert(owner->Finish() == Status::OutcomeUnknown);
    assert(fstatat(state.directory, kStore, &value, AT_SYMLINK_NOFOLLOW) == 0 &&
           state.file_id.Same(value));
    assert(fstatat(state.directory, "extra", &value, AT_SYMLINK_NOFOLLOW) == 0 &&
           state.file_id.Same(value));
    RemoveFixture(std::move(owner));
  }
  static void RetryFlush() {
    auto owner = Setup(true);
    const std::string parent = owner->state_->bootstrap.canonical_parent;
    fail_flush = true;
    assert(owner->Finish() == Status::OutcomeUnknown);
    assert(owner->state_->cleanup == OwnedIdentity::State::Cleanup::DirectoryRemoved);
    assert(owner->Finish() == Status::Ready);
    assert(owner->Finish() == Status::Ready);
    owner.reset();
    assert(rmdir(parent.c_str()) == 0);
  }
  static void EarlyDrop() {
    auto owner = Setup(true);
    const std::string parent = owner->state_->bootstrap.canonical_parent;
    const std::string directory = owner->state_->directory_path;
    owner.reset();
    struct stat value{};
    assert(lstat((directory + "/" + kStore).c_str(), &value) == 0);
    assert(unlink((directory + "/" + kStore).c_str()) == 0);
    assert(rmdir(directory.c_str()) == 0 && rmdir(parent.c_str()) == 0);
  }
  static void ParentAclChange() {
    auto owner = Setup(true);
    acl_t acl = acl_init(1); assert(acl);
    acl_entry_t entry{}; assert(acl_create_entry(&acl, &entry) == 0);
    assert(acl_set_tag_type(entry, ACL_EXTENDED_ALLOW) == 0);
    uuid_t qualifier{}; assert(mbr_uid_to_uuid(geteuid(), qualifier) == 0);
    assert(acl_set_qualifier(entry, qualifier) == 0);
    acl_permset_t permissions{}; assert(acl_get_permset(entry, &permissions) == 0);
    assert(acl_add_perm(permissions, ACL_READ_DATA) == 0);
    assert(acl_set_fd_np(owner->state_->parent, acl, ACL_TYPE_EXTENDED) == 0); acl_free(acl);
    assert(!owner->state_->ParentValid());
    assert(owner->Finish() == Status::OutcomeUnknown);
    RemoveFixture(std::move(owner));
  }
  static void PasswordDeniedWipes() {
    std::array<std::uint8_t, 4> password{'t', 'e', 's', 't'};
    Enrollment enrollment;
    enrollment.password = password;
    std::unique_ptr<OwnedIdentity> output;
    // Invalid retained descriptor rejects before any Security API.
    assert(OwnedIdentity::Prepare({}, std::move(enrollment), &output) == Status::Denied);
    assert(!output && std::all_of(password.begin(), password.end(), [](auto byte) { return byte == 0; }));
  }
  static void CancellationAndRevoke() {
    auto owner = Setup(true);
    owner->state_->ready = true;
    const Generation generation = owner->state_->bootstrap.generation;
    Generation stale = generation;
    stale[0] ^= 1;
    owner->state_->handshake = std::pair{std::uint64_t{1}, std::chrono::steady_clock::now()};
    owner->CancelHandshake({1, stale}); assert(owner->state_->handshake);
    owner->CancelHandshake({2, generation}); assert(owner->state_->handshake);
    owner->CancelHandshake({1, generation}); assert(!owner->state_->handshake);
    owner->state_->handshake = std::pair{std::uint64_t{2}, std::chrono::steady_clock::now()};
    owner->Revoke(); assert(!owner->state_->ready && !owner->state_->handshake);
    assert(std::all_of(owner->state_->bootstrap.generation.begin(),
                       owner->state_->bootstrap.generation.end(),
                       [](auto byte) { return byte == 0; }));
    RemoveFixture(std::move(owner));
  }
  static void FinishRetiresGeneration() {
    auto owner = Setup(true);
    const std::string parent = owner->state_->bootstrap.canonical_parent;
    assert(owner->Finish() == Status::Ready);
    assert(std::all_of(owner->state_->bootstrap.generation.begin(),
                       owner->state_->bootstrap.generation.end(),
                       [](auto byte) { return byte == 0; }));
    owner.reset();
    assert(rmdir(parent.c_str()) == 0);
  }
  static void LoginNamespaceRejected() {
    assert(forbidden_parent("/private/tmp/login.keychain-owned"));
    assert(forbidden_parent("/private/tmp/Login.Keychain-db/parent"));
    assert(forbidden_parent("/private/tmp/System.Keychain/parent"));
    assert(!forbidden_parent("/private/tmp/colossus-owned-pki"));
  }
  static void GenerationAndOriginBinding() {
    auto owner = Setup(true);
    auto& state = *owner->state_;
    Generation stale = state.bootstrap.generation;
    stale[0] ^= 1;
    Fingerprint other_leaf = state.leaf_hash;
    other_leaf[0] ^= 1;
    assert(state.BindingValid(state.bootstrap.generation,
                              "https://allowed.example", state.leaf_hash));
    assert(!state.BindingValid(stale, "https://allowed.example", state.leaf_hash));
    assert(!state.BindingValid(state.bootstrap.generation,
                               "https://other.example", state.leaf_hash));
    assert(!state.BindingValid(state.bootstrap.generation,
                               "https://allowed.example", other_leaf));
    RemoveFixture(std::move(owner));
  }
  struct Window final : MutationWindow {
    bool live = true, accepted = true;
    unsigned int receipts = 0;
    bool Exclusive() const override { return live; }
    bool Seal(std::uint64_t sequence, std::span<const int> descriptors) override {
      assert(sequence == 1 && descriptors.size() == 2);
      ++receipts;
      return accepted;
    }
    bool Checkpoint(std::uint64_t, std::uint32_t) override { return false; }
    bool RetireUnchanged(std::uint64_t,std::span<const int>) override {return false;}
  };
  static std::string CreateAux(OwnedIdentity::State& state) {
    std::array<std::uint8_t, CC_SHA1_DIGEST_LENGTH> digest{};
    assert(CC_SHA1(state.store_name.data(), state.store_name.size(), digest.data()));
    char name[12]; snprintf(name, sizeof(name), ".fl%02X%02X%02X%02X", digest[0], digest[1], digest[2], digest[3]);
    const int file = openat(state.directory, name, O_CREAT | O_EXCL | O_WRONLY | O_NOFOLLOW | O_CLOEXEC, 0444);
    assert(file >= 0); close(file);
    return name;
  }
  static void PositiveReceiptAndProviderReplacement() {
    auto owner = Setup(true);
    auto& state = *owner->state_;
    state.mutation_sequence = 1;
    const auto name = CreateAux(state);
    Window window;
    assert(state.CaptureOutputs(window, true, state.store_path));
    assert(window.receipts == 1 && state.auxiliary_recorded && state.AuxiliaryBound());
    const FileIdentity original = state.file_id;
    assert(unlinkat(state.directory, kStore, 0) == 0);
    const int replacement = openat(state.directory, kStore, O_CREAT | O_EXCL | O_WRONLY | O_NOFOLLOW | O_CLOEXEC, 0644);
    assert(replacement >= 0); close(replacement);
    assert(!state.FileBound());
    assert(state.CaptureOutputs(window, false, state.store_path));
    struct stat metadata{};
    assert(fstat(state.file, &metadata) == 0 && !original.Same(metadata) && (metadata.st_mode & 0077) == 0);
    const auto parent = state.bootstrap.canonical_parent;
    assert(owner->Finish() == Status::Ready);
    owner.reset(); assert(rmdir(parent.c_str()) == 0);
  }
  static void AbsentExclusionAndRejectedReceipt() {
    auto owner = Setup(true);
    auto& state = *owner->state_;
    state.mutation_sequence = 1;
    const auto name = CreateAux(state);
    const FileIdentity original = state.file_id;
    Window window; window.live = false;
    assert(!state.CaptureOutputs(window, true, state.store_path) && window.receipts == 0);
    window.live = true; window.accepted = false;
    assert(!state.CaptureOutputs(window, true, state.store_path) && window.receipts == 1);
    assert(!state.auxiliary_recorded && state.file_id.inode == original.inode);
    state.creation_unknown = true;
    assert(owner->Finish() == Status::OutcomeUnknown);
    assert(unlinkat(state.directory, name.c_str(), 0) == 0);
    RemoveFixture(std::move(owner));
  }
  static void ForeignAuxGenerationIsPreserved() {
    auto owner = Setup(true);
    auto& state = *owner->state_;
    state.mutation_sequence = 1;
    const auto name = CreateAux(state);
    Window window;
    assert(state.CaptureOutputs(window, true, state.store_path));
    assert(renameat(state.directory, name.c_str(), state.directory, "keep-aux") == 0);
    const int replacement = openat(state.directory, name.c_str(), O_CREAT | O_EXCL | O_WRONLY | O_NOFOLLOW | O_CLOEXEC, 0444);
    assert(replacement >= 0); close(replacement);
    assert(!state.CaptureOutputs(window, false, state.store_path));
    assert(owner->Finish() == Status::OutcomeUnknown);
    RemoveFixture(std::move(owner));
  }
  static void CleanupRequiresIndependentExclusion() {
    auto owner = Setup(true);
    auto& state = *owner->state_;
    auto authority = std::static_pointer_cast<CleanupAuthority>(state.bootstrap.mutations);
    authority->enabled = false;
    owner->Revoke();
    assert(owner->Finish() == Status::OutcomeUnknown);
    assert(state.cleanup == OwnedIdentity::State::Cleanup::Released && state.FileBound());
    authority->enabled = true;
    const auto parent = state.bootstrap.canonical_parent;
    assert(owner->Finish() == Status::Ready);
    assert(std::all_of(state.custody_generation.begin(), state.custody_generation.end(), [](auto byte) { return byte == 0; }));
    owner.reset(); assert(rmdir(parent.c_str()) == 0);
  }
  struct NamespaceWindow final : MutationWindow {
    bool accepted;
    explicit NamespaceWindow(bool receipt) : accepted(receipt) {}
    bool Exclusive() const override { return true; }
    bool Seal(std::uint64_t, std::span<const int> outputs) override {
      struct stat metadata{};
      assert(outputs.size() == 1 && fstat(outputs[0], &metadata) == 0 && private_metadata(metadata, true));
      return accepted;
    }
    bool Checkpoint(std::uint64_t, std::uint32_t) override { return false; }
    bool RetireUnchanged(std::uint64_t,std::span<const int>) override {return false;}
  };
  struct NamespaceAuthority final : MutationAuthority {
    bool admitted = true, accepted = true;
    bool SigningAdmitted(const Generation&) const override { return false; }
    std::unique_ptr<MutationWindow> Begin(const Generation&, int parent, int directory,
        std::uint64_t, Mutation operation) override {
      assert(parent >= 0 && directory == -1 && operation == Mutation::CreateNamespace);
      return admitted ? std::make_unique<NamespaceWindow>(accepted) : nullptr;
    }
  };
  struct SigningAuthority final : MutationAuthority {
    bool parent_alive = true, package_current = true;
    bool SigningAdmitted(const Generation& generation) const override {
      return parent_alive && package_current && generation == CleanupAuthority::GenerationFilled();
    }
    std::unique_ptr<MutationWindow> Begin(const Generation&, int, int,
        std::uint64_t, Mutation) override { return {}; }
  };
  static void ParentLossImmediatelyRetiresSigningGeneration() {
    OwnedIdentity::State state;
    auto authority = std::make_shared<SigningAuthority>();
    state.bootstrap.mutations = authority;
    state.bootstrap.generation = CleanupAuthority::GenerationFilled();
    state.custody_generation = state.bootstrap.generation;
    state.ready = true;
    state.handshake = std::pair{std::uint64_t{1}, std::chrono::steady_clock::now() + std::chrono::seconds(2)};
    assert(state.SigningAdmitted());
    authority->parent_alive = false;
    // Valid checks admission before any native store/code/Security calls.
    assert(!state.Valid() && !state.ready && !state.handshake && !state.changed);
    assert(std::all_of(state.bootstrap.generation.begin(), state.bootstrap.generation.end(), [](auto byte) { return byte == 0; }));
    assert(state.custody_generation == CleanupAuthority::GenerationFilled());
    authority->parent_alive = true;
    assert(!state.SigningAdmitted());
  }
  static void NamespaceCreationRequiresAnIndependentReceipt() {
    for (const int scenario : {0, 1, 2}) {
      const bool admitted = scenario != 0, accepted = scenario == 2;
      auto owner = Setup(false);
      auto& state = *owner->state_;
      assert(unlinkat(state.directory, kStore, 0) == 0);
      assert(unlinkat(state.parent, kDirectory, AT_REMOVEDIR) == 0);
      close(state.directory); state.directory = -1; state.created_directory = false;
      auto authority = std::make_shared<NamespaceAuthority>();
      authority->admitted = admitted; authority->accepted = accepted;
      state.bootstrap.mutations = authority;
      assert(state.CreateNamespace() == (accepted ? Status::Ready : admitted ? Status::OutcomeUnknown : Status::Denied));
      struct stat metadata{};
      if (accepted) {
        assert(!state.creation_unknown && state.NamespaceValid());
        state.bootstrap.mutations = std::make_shared<CleanupAuthority>();
        const auto parent = state.bootstrap.canonical_parent;
        assert(owner->Finish() == Status::Ready);
        owner.reset(); assert(rmdir(parent.c_str()) == 0);
      } else if (admitted) {
        assert(state.creation_unknown && state.NamespaceValid());
        assert(owner->Finish() == Status::OutcomeUnknown);
        RemoveFixture(std::move(owner));
      } else {
        assert(fstatat(state.parent, kDirectory, &metadata, AT_SYMLINK_NOFOLLOW) != 0 && errno == ENOENT);
        const auto parent = state.bootstrap.canonical_parent;
        assert(owner->Finish() == Status::Ready);
        owner.reset(); assert(rmdir(parent.c_str()) == 0);
      }
    }
  }
};
}  // namespace colossus::browser::pki::mac

int main() {
  using Fixture = colossus::browser::pki::mac::FileOnlyFixture;
  for (auto test : {Fixture::UnknownCreation, Fixture::ExtraEntry, Fixture::ReplacedStore,
                    Fixture::DerivedLockRejected, Fixture::StoreAliasRejected,
                    Fixture::RetryFlush, Fixture::EarlyDrop, Fixture::ParentAclChange,
                    Fixture::PasswordDeniedWipes, Fixture::CancellationAndRevoke,
                    Fixture::LoginNamespaceRejected, Fixture::GenerationAndOriginBinding,
                    Fixture::FinishRetiresGeneration, Fixture::PositiveReceiptAndProviderReplacement,
                    Fixture::AbsentExclusionAndRejectedReceipt, Fixture::ForeignAuxGenerationIsPreserved,
                    Fixture::CleanupRequiresIndependentExclusion,
                    Fixture::ParentLossImmediatelyRetiresSigningGeneration,
                    Fixture::NamespaceCreationRequiresAnIndependentReceipt}) {
    const pid_t child = fork(); assert(child >= 0);
    if (child == 0) { test(); _exit(0); }
    int status = 0;
    assert(waitpid(child, &status, 0) == child && WIFEXITED(status) && WEXITSTATUS(status) == 0);
  }
  puts("PASS nineteen file/memory-only owned PKI custody regressions; no Security API, audit-port, or browser calls");
}
