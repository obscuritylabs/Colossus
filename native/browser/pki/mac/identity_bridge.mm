#include "identity_bridge.h"
#include <CommonCrypto/CommonDigest.h>
#include <Security/Security.h>
#include <limits.h>
#include "owned_identity.h"
#include <algorithm>
#include <cstring>
#include <new>
namespace {
using namespace colossus::browser::pki::mac;
uint32_t status(Status value) { return static_cast<uint32_t>(value); }
struct Window final : MutationWindow {
  ColossusMutationCallbacks callbacks;
  uint64_t sequence;
  Window(ColossusMutationCallbacks value, uint64_t seq) : callbacks(value), sequence(seq) {}
  ~Window() { callbacks.end(callbacks.context, sequence); }
  bool Exclusive() const override { return callbacks.exclusive(callbacks.context, sequence); }
  bool Seal(uint64_t seq, std::span<const int> fds) override {
    return seq == sequence && callbacks.seal(callbacks.context, sequence, fds.data(), fds.size());
  }
  bool Checkpoint(uint64_t seq, uint32_t step) override {
    return seq == sequence && callbacks.checkpoint(callbacks.context, sequence, step);
  }
  bool RetireUnchanged(uint64_t seq,std::span<const int> fds) override {
    return seq==sequence&&callbacks.retire_unchanged(callbacks.context,sequence,fds.data(),fds.size());
  }
};
struct Authority final : MutationAuthority {
  ColossusMutationCallbacks callbacks;
  explicit Authority(ColossusMutationCallbacks value) : callbacks(value) {}
  bool SigningAdmitted(const Generation& generation) const override {
    return callbacks.signing_admitted(callbacks.context, generation.data());
  }
  std::unique_ptr<MutationWindow> Begin(const Generation& generation, int parent, int directory, uint64_t sequence, Mutation mutation) override {
    if (!callbacks.begin(callbacks.context, generation.data(), parent, directory, sequence, static_cast<uint32_t>(mutation))) return {};
    return std::make_unique<Window>(callbacks, sequence);
  }
};
struct Bridge { std::unique_ptr<OwnedIdentity> owner; };
std::string text(ColossusPublicBytes bytes) { return {reinterpret_cast<const char*>(bytes.data), bytes.size}; }
std::vector<uint8_t> vector(ColossusPublicBytes bytes) { if (!bytes.size) return {}; return {bytes.data, bytes.data + bytes.size}; }
bool ca(std::span<const uint8_t> bytes) { return colossus_mac_validate_ca(bytes.data(), bytes.size()); }
bool identity(std::span<const uint8_t> bytes) { return colossus_mac_validate_identity(bytes.data(), bytes.size()); }
bool origin(const std::string& value) { return colossus_mac_validate_origin(reinterpret_cast<const uint8_t*>(value.data()), value.size()); }
Generation generation(const uint8_t* bytes) { Generation value{}; memcpy(value.data(), bytes, value.size()); return value; }
}
extern "C" uint32_t colossus_mac_identity_prepare(const ColossusIdentityEnrollment* input, void** output) {
  if (!input || !output || *output || !input->code_hash || !input->generation || !input->mutations.context || !input->mutations.begin || !input->mutations.exclusive || !input->mutations.seal || !input->mutations.end || !input->mutations.signing_admitted || !input->mutations.checkpoint || !input->mutations.retire_unchanged) return status(Status::Denied);
  auto bridge = std::make_unique<Bridge>();
  Bootstrap bootstrap;
  bootstrap.retained_parent_fd = input->parent_fd;
  bootstrap.canonical_parent = text(input->parent_path);
  memcpy(bootstrap.expected_broker_cdhash.data(), input->code_hash, bootstrap.expected_broker_cdhash.size());
  bootstrap.generation = generation(input->generation);
  bootstrap.validate = {ca, identity, origin};
  bootstrap.mutations = std::make_shared<Authority>(input->mutations);
  Enrollment enrollment;
  enrollment.reviewed_leaf_der = vector(input->reviewed_leaf);
  Fingerprint leaf{};
  // Native Prepare computes and validates the actual reviewed leaf hash. The
  // bridge binding receives that same hash from the Rust validation boundary.
  // Copy it from the public certificate using the same SHA256 native primitive.
  CC_SHA256(enrollment.reviewed_leaf_der.data(), static_cast<CC_LONG>(enrollment.reviewed_leaf_der.size()), leaf.data());
  bootstrap.bindings.push_back({text(input->origin), leaf});
  for (size_t index = 0; index < input->ca_count; ++index) enrollment.ca_der.push_back(vector(input->ca[index]));
  enrollment.encrypted_pkcs12 = {input->encrypted_pkcs12.data, input->encrypted_pkcs12.size};
  enrollment.password = {input->password, input->password_size};
  const auto result = OwnedIdentity::Prepare(std::move(bootstrap), std::move(enrollment), &bridge->owner);
  if (bridge->owner) *output = bridge.release();
  return status(result);
}
extern "C" uint32_t colossus_mac_identity_begin(void* raw, const uint8_t* gen, ColossusPublicBytes origin_bytes, const uint8_t* leaf_bytes, uint64_t milliseconds, uint64_t* sequence) {
  if (!raw || !gen || !leaf_bytes || !sequence || milliseconds == 0 || milliseconds > 30000) return status(Status::Denied);
  Fingerprint leaf{}; memcpy(leaf.data(), leaf_bytes, leaf.size()); Handshake handshake;
  const auto result = static_cast<Bridge*>(raw)->owner->BeginHandshake(generation(gen), text(origin_bytes), leaf, std::chrono::steady_clock::now() + std::chrono::milliseconds(milliseconds), &handshake);
  if (result == Status::Ready) *sequence = handshake.sequence;
  return status(result);
}
extern "C" uint32_t colossus_mac_identity_verify(void* raw, uint64_t sequence, const uint8_t* gen, uint32_t scheme, const uint8_t* hash, uint8_t* output, size_t* length) {
  if (!raw || !gen || !hash || !output || !length || *length < 512) return status(Status::Denied);
  std::array<uint8_t, 32> transcript{}; memcpy(transcript.data(), hash, transcript.size());
  std::vector<uint8_t> signature;
  const auto result = static_cast<Bridge*>(raw)->owner->CertificateVerify({sequence, generation(gen)}, static_cast<Tls13Scheme>(scheme), transcript, &signature);
  *length = 0;
  if (result == Status::Ready && signature.size() <= 512) { memcpy(output, signature.data(), signature.size()); *length = signature.size(); }
  return status(result);
}
extern "C" void colossus_mac_identity_cancel(void* raw, uint64_t sequence, const uint8_t* gen) { if (raw && gen) static_cast<Bridge*>(raw)->owner->CancelHandshake({sequence, generation(gen)}); }
extern "C" void colossus_mac_identity_revoke(void* raw) { if (raw) static_cast<Bridge*>(raw)->owner->Revoke(); }
extern "C" uint32_t colossus_mac_identity_finish(void* raw) { return raw ? status(static_cast<Bridge*>(raw)->owner->Finish()) : status(Status::Denied); }
extern "C" bool colossus_mac_identity_diagnostic(void* raw, uint32_t* phase, uint32_t* check, int32_t* os_status) {
  if (!raw || !phase || !check || !os_status) return false;
  const auto diagnostic = static_cast<Bridge*>(raw)->owner->CopyDiagnostic();
  *phase = static_cast<uint32_t>(diagnostic.phase); *check = static_cast<uint32_t>(diagnostic.check); *os_status = diagnostic.os_status;
  return true;
}
extern "C" void colossus_mac_identity_drop(void* raw) { delete static_cast<Bridge*>(raw); }

#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
extern "C" bool colossus_mac_keychain_metadata(uint8_t* output) {
  if(!output)return false;CC_SHA256_CTX context{};if(!CC_SHA256_Init(&context))return false;
  const auto path=[&context](SecKeychainRef keychain){char value[PATH_MAX];UInt32 size=sizeof(value);
    if(!keychain||SecKeychainGetPath(keychain,&size,value)!=errSecSuccess||size>=sizeof(value))return false;
    return CC_SHA256_Update(&context,&size,sizeof(size))&&CC_SHA256_Update(&context,value,size);};
  SecKeychainRef selected=nullptr;const OSStatus status=SecKeychainCopyDefault(&selected);
  bool valid=CC_SHA256_Update(&context,&status,sizeof(status));
  if(status==errSecSuccess)valid=valid&&path(selected);
  else valid=valid&&status==errSecNoDefaultKeychain;
  if(selected)CFRelease(selected);
  CFArrayRef list=nullptr;const OSStatus searched=SecKeychainCopySearchList(&list);
  valid=valid&&searched==errSecSuccess&&list&&CFGetTypeID(list)==CFArrayGetTypeID();
  CFIndex count=valid?CFArrayGetCount(list):0;valid=valid&&count>=0&&count<=256&&CC_SHA256_Update(&context,&count,sizeof(count));
  for(CFIndex index=0;valid&&index<count;++index){CFTypeRef item=CFArrayGetValueAtIndex(list,index);valid=item&&CFGetTypeID(item)==SecKeychainGetTypeID()&&path(static_cast<SecKeychainRef>(const_cast<void*>(item)));}
  if(list)CFRelease(list);return valid&&CC_SHA256_Final(output,&context);
}
#pragma clang diagnostic pop
