#include "profile_crypto.h"
#if !defined(COLOSSUS_MAC_PROFILE_CRYPTO_DEVELOPMENT) || defined(NDEBUG)
#error "Owned macOS profile crypto is restricted to explicit Debug development builds"
#endif
#import <Security/Security.h>
#include <dlfcn.h>
#include <sys/acl.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <array>
#include <cerrno>
#include <cstring>
#include <limits.h>
#include <mutex>
#include <string>
#include <vector>
#include <dirent.h>
#include <cstdio>
#include <utility>
#include <CommonCrypto/CommonDigest.h>

// File Keychain APIs and the three weak-resolved legacy item SPIs are used only
// by this explicitly unsupported developer component. No deprecated API or
// interposing acceptance is advertised by a normal or release build.
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"

namespace {
constexpr char kDirectory[] = "colossus-profile-crypto";
constexpr char kStore[] = "colossus-profile-crypto.keychain";
constexpr char kService[] = "Chromium Safe Storage";
constexpr char kAccount[] = "Chromium";
constexpr size_t kSecretBytes = 32;

template<class T> struct Ref {
  T value = nullptr;
  Ref()=default;
  Ref(const Ref&)=delete; Ref& operator=(const Ref&)=delete;
  ~Ref() { if (value) CFRelease(value); }
  T* out() { return &value; }
  T release() { T result=value; value=nullptr; return result; }
};
struct Identity {
  dev_t device{}; ino_t inode{}; uid_t uid{};
  bool same(const struct stat& value) const {
    return device==value.st_dev && inode==value.st_ino && uid==value.st_uid;
  }
};
void wipe(void* data,size_t size) {
  volatile unsigned char* bytes=static_cast<volatile unsigned char*>(data);
  while (size--) *bytes++=0;
}
struct WipeOnExit {
  void* data;
  size_t size;
  ~WipeOnExit() {wipe(data,size);}
};
Identity identity(const struct stat& value) {
  return {value.st_dev,value.st_ino,value.st_uid};
}
bool private_metadata(const struct stat& value, bool directory) {
  return value.st_uid==geteuid() && !(value.st_mode & 0077) &&
      (directory ? S_ISDIR(value.st_mode) : S_ISREG(value.st_mode) && value.st_nlink==1);
}
bool no_extended_acl(int fd) {
  // acl_get_fd_np reports ENOENT for an absent ACL on APFS. Query the file
  // security property explicitly so absence is distinguished from stat failure.
  filesec_t security=filesec_init();
  if (!security) return false;
  struct stat metadata{};int present=0;bool empty=false;
  if (fstatx_np(fd,&metadata,security)==0 &&
      filesec_query_property(security,FILESEC_ACL,&present)==0) {
    if (!present) empty=true;
    else {
      acl_t acl=nullptr;
      if (filesec_get_property(security,FILESEC_ACL,&acl)==0 && acl) {
        acl_entry_t entry{};errno=0;
        const int result=acl_get_entry(acl,ACL_FIRST_ENTRY,&entry);
        const int error=errno;
        empty=acl_valid(acl)==0 && result==-1 && error==EINVAL;
        acl_free(acl);
      }
    }
  }
  filesec_free(security);return empty;
}
bool canonical(const std::string& path) {
  char actual[PATH_MAX];
  return realpath(path.c_str(),actual) && path==actual;
}
bool forbidden_name(const std::string& path) {
  std::string folded(path);
  for (char& ch : folded) if (ch>='A' && ch<='Z') ch+=('a'-'A');
  return folded.find("/login.keychain")!=std::string::npos ||
      folded.find("/system.keychain")!=std::string::npos;
}
using CreateNew=OSStatus(*)(SecItemClass,OSType,UInt32,const void*,SecKeychainItemRef*);
using SetAttribute=OSStatus(*)(SecKeychainItemRef,SecKeychainAttribute*);
using AddNoUI=OSStatus(*)(SecKeychainRef,SecKeychainItemRef);
using TrustedFromRequirement=OSStatus(*)(const char*,SecRequirementRef,SecTrustedApplicationRef*);
using TrustedCopyRequirement=OSStatus(*)(SecTrustedApplicationRef,SecRequirementRef*);
enum class Cleanup {Live,Locked,Released,Retired,FileRemoved,AuxRemoved,DirectoryRemoved,Finished};
struct State {
  std::mutex mutex;
  int32_t phase=COLOSSUS_MAC_PROFILE_PHASE_NOT_ATTEMPTED;
  bool attempted=false, ready=false, changed=false, cleaned=false, routing=false, directory_created=false, creation_unknown=false, store_recorded=false;
  bool fixture_store_ready=false, aux_recorded=false;
  Cleanup cleanup=Cleanup::Live;
  int parent=-1, directory=-1, file=-1, aux=-1;
  Identity parent_id{}, directory_id{}, file_id{}, aux_id{};
  std::string parent_path, directory_path, store_path, store_name, aux_name, retired_name;
  SecKeychainRef keychain=nullptr;
  SecKeychainItemRef item=nullptr;
  SecCodeRef code=nullptr;
  SecRequirementRef requirement=nullptr;
  CFDataRef code_hash=nullptr;
  CreateNew create_new=nullptr; SetAttribute set_attribute=nullptr; AddNoUI add_no_ui=nullptr;
  TrustedFromRequirement trusted_from=nullptr; TrustedCopyRequirement trusted_requirement=nullptr;
};
// No atexit store mutation: an unknown teardown must survive for reconciliation.
State& state() { static State* value=new State; return *value; }
thread_local bool owned_api=false;
struct OwnedCall { bool previous=owned_api; OwnedCall(){owned_api=true;} ~OwnedCall(){owned_api=previous;} };

bool code_valid(State& owner) {
  if (!owner.code || !owner.requirement || !owner.code_hash ||
      SecCodeCheckValidity(owner.code,kSecCSDefaultFlags,owner.requirement)!=errSecSuccess)
    return false;
  Ref<SecStaticCodeRef> code;
  Ref<CFDictionaryRef> information;
  if (SecCodeCopyStaticCode(owner.code,kSecCSDefaultFlags,code.out())!=errSecSuccess ||
      SecCodeCopySigningInformation(code.value,kSecCSSigningInformation,information.out())!=errSecSuccess)
    return false;
  CFTypeRef hash=CFDictionaryGetValue(information.value,kSecCodeInfoUnique);
  return hash && CFGetTypeID(hash)==CFDataGetTypeID() && CFEqual(hash,owner.code_hash);
}
bool parent_valid(State& owner) {
  struct stat parent{}, entry{};
  return owner.parent>=0 && fstat(owner.parent,&parent)==0 && owner.parent_id.same(parent) &&
      private_metadata(parent,true) && no_extended_acl(owner.parent) &&
      lstat(owner.parent_path.c_str(),&entry)==0 && owner.parent_id.same(entry) &&
      private_metadata(entry,true) && canonical(owner.parent_path);
}
bool namespace_valid(State& owner) {
  struct stat directory{}, entry{};
  const bool retired=owner.cleanup>=Cleanup::Retired;
  const std::string name=retired ? owner.retired_name : kDirectory;
  const std::string path=owner.parent_path+"/"+name;
  return parent_valid(owner) && owner.directory>=0 && !name.empty() &&
      fstat(owner.directory,&directory)==0 && owner.directory_id.same(directory) &&
      private_metadata(directory,true) && no_extended_acl(owner.directory) &&
      fstatat(owner.parent,name.c_str(),&entry,AT_SYMLINK_NOFOLLOW)==0 &&
      owner.directory_id.same(entry) && private_metadata(entry,true) && canonical(path);
}
bool file_bound(State& owner) {
  struct stat file{}, entry{};
  const std::string directory=owner.parent_path+"/"+(owner.cleanup>=Cleanup::Retired ? owner.retired_name : kDirectory);
  return owner.store_recorded && owner.file>=0 && namespace_valid(owner) &&
      fstat(owner.file,&file)==0 && owner.file_id.same(file) && private_metadata(file,false) && no_extended_acl(owner.file) &&
      fstatat(owner.directory,owner.store_name.c_str(),&entry,AT_SYMLINK_NOFOLLOW)==0 &&
      owner.file_id.same(entry) && private_metadata(entry,false) && canonical(directory+"/"+owner.store_name);
}
bool aux_bound(State& owner) {
  struct stat file{},entry{};
  const std::string directory=owner.parent_path+"/"+(owner.cleanup>=Cleanup::Retired ? owner.retired_name : kDirectory);
  return owner.aux_recorded && owner.aux>=0 && namespace_valid(owner) &&
      fstat(owner.aux,&file)==0 && owner.aux_id.same(file) && private_metadata(file,false) &&
      file.st_size==0 && no_extended_acl(owner.aux) &&
      fstatat(owner.directory,owner.aux_name.c_str(),&entry,AT_SYMLINK_NOFOLLOW)==0 &&
      owner.aux_id.same(entry) && private_metadata(entry,false) && entry.st_size==0 &&
      canonical(directory+"/"+owner.aux_name);
}
bool exact_entries(State& owner,bool expect_store) {
  int fd=fcntl(owner.directory,F_DUPFD_CLOEXEC,3);
  if (fd<0) return false;
  DIR* entries=fdopendir(fd); if (!entries) {close(fd);return false;}
  rewinddir(entries);
  const bool expect_aux=owner.aux_recorded && owner.cleanup<Cleanup::AuxRemoved;
  unsigned int count=0,seen=0; bool safe=true;
  while (true) {
    errno=0; dirent* entry=readdir(entries);
    if (!entry) {if(errno) safe=false;break;}
    std::string name=entry->d_name;
    if (name=="." || name=="..") continue;
    const unsigned int bit=expect_store && name==owner.store_name ? 1 :
        expect_aux && name==owner.aux_name ? 2 : 0;
    if (!bit || (seen&bit)) {safe=false;break;}
    seen|=bit;++count;
    struct stat current{};
    if (fstatat(owner.directory,name.c_str(),&current,AT_SYMLINK_NOFOLLOW)!=0 ||
        !(bit==1 ? owner.file_id.same(current) : owner.aux_id.same(current)) ||
        !private_metadata(current,false) || (bit==2 && current.st_size!=0)) {safe=false;break;}
  }
  closedir(entries);
  return safe && count==(expect_store ? 1u : 0u)+(expect_aux ? 1u : 0u);
}
bool store_bound(State& owner) {
  if (owner.cleanup!=Cleanup::Live || !owner.keychain || !file_bound(owner) ||
      (owner.aux_recorded && !aux_bound(owner)) || !exact_entries(owner,true)) return false;
  char path[PATH_MAX]; UInt32 length=sizeof(path);
  return SecKeychainGetPath(owner.keychain,&length,path)==errSecSuccess &&
      length<sizeof(path) && owner.store_path==std::string(path,length);
}
bool store_valid(State& owner) {
  SecKeychainStatus status=0;
  return store_bound(owner) && SecKeychainGetStatus(owner.keychain,&status)==errSecSuccess &&
      (status & kSecUnlockStateStatus);
}
bool acl_valid_for_host(State& owner);
bool same_item_store(State& owner) {
  if (!owner.item) return false;
  Ref<SecKeychainRef> keychain;
  if (SecKeychainItemCopyKeychain(owner.item,keychain.out())!=errSecSuccess || !keychain.value) return false;
  char path[PATH_MAX]; UInt32 length=sizeof(path);
  return SecKeychainGetPath(keychain.value,&length,path)==errSecSuccess && length<sizeof(path) &&
      owner.store_path==std::string(path,length);
}
bool safe_query(CFDictionaryRef query) {
  if (!query || CFGetTypeID(query)!=CFDictionaryGetTypeID() || CFDictionaryGetCount(query)!=6) return false;
  const void* keys[]={kSecClass,kSecAttrService,kSecAttrAccount,kSecMatchLimit,kSecReturnData,kSecReturnAttributes};
  const void* values[]={kSecClassGenericPassword,CFSTR("Chromium Safe Storage"),CFSTR("Chromium"),
      kSecMatchLimitOne,kCFBooleanTrue,kCFBooleanTrue};
  for (size_t n=0;n<6;++n) {
    const void* value=CFDictionaryGetValue(query,keys[n]);
    if (!value || !CFEqual(value,values[n])) return false;
  }
  return true;
}
OSStatus scoped_copy(CFDictionaryRef query,CFTypeRef* result) {
  if (result) *result=nullptr;
  if (owned_api || !result || !safe_query(query)) return errSecNotAvailable;
  State& owner=state(); std::lock_guard lock(owner.mutex); OwnedCall call;
  if (!owner.ready || owner.changed || !owner.routing) return errSecNotAvailable;
  if (!code_valid(owner) || !store_bound(owner) || !same_item_store(owner) || !acl_valid_for_host(owner)) {
    owner.ready=false; owner.changed=true; return errSecNotAvailable;
  }
  if (!store_valid(owner)) {owner.ready=false; return errSecNotAvailable;}
  UInt32 tags[]={kSecServiceItemAttr,kSecAccountItemAttr};
  UInt32 formats[]={CSSM_DB_ATTRIBUTE_FORMAT_BLOB,CSSM_DB_ATTRIBUTE_FORMAT_BLOB};
  SecKeychainAttributeInfo info{2,tags,formats};
  SecItemClass item_class{}; SecKeychainAttributeList* attributes=nullptr;
  UInt32 length=0; void* data=nullptr;
  OSStatus status=SecKeychainItemCopyAttributesAndData(owner.item,&info,&item_class,&attributes,&length,&data);
  bool valid=status==errSecSuccess && item_class==kSecGenericPasswordItemClass &&
      length==kSecretBytes && data && attributes && attributes->count==2;
  if (valid) {
    unsigned int seen=0;
    for (UInt32 n=0;n<2;++n) {
      auto& attribute=attributes->attr[n];
      const unsigned int bit=attribute.tag==kSecServiceItemAttr ? 1 : attribute.tag==kSecAccountItemAttr ? 2 : 0;
      if (!bit || (seen & bit)) valid=false;
      seen|=bit;
      const char* expected=attribute.tag==kSecServiceItemAttr ? kService :
          attribute.tag==kSecAccountItemAttr ? kAccount : nullptr;
      if (!expected || attribute.length!=strlen(expected) ||
          !attribute.data || memcmp(attribute.data,expected,attribute.length)!=0) valid=false;
    }
    if (seen!=3) valid=false;
  }
  valid=valid && store_valid(owner) && same_item_store(owner) && code_valid(owner) && acl_valid_for_host(owner);
  if (valid) {
    Ref<CFDataRef> secret;
    secret.value=CFDataCreate(kCFAllocatorDefault,static_cast<const UInt8*>(data),length);
    if (secret.value) {
      const void* keys[]={kSecClass,kSecAttrService,kSecAttrAccount,kSecValueData};
      const void* values[]={kSecClassGenericPassword,CFSTR("Chromium Safe Storage"),CFSTR("Chromium"),secret.value};
      *result=CFDictionaryCreate(kCFAllocatorDefault,keys,values,4,&kCFTypeDictionaryKeyCallBacks,&kCFTypeDictionaryValueCallBacks);
    }
  }
  if (data && length) wipe(data,length);
  if (attributes || data) SecKeychainItemFreeAttributesAndData(attributes,data);
  if (!valid) { owner.ready=false; owner.changed=true; }
  return valid && *result ? errSecSuccess : errSecNotAvailable;
}
OSStatus deny_add(CFDictionaryRef,CFTypeRef* result) {
  if (result) *result=nullptr;
  return errSecNotAvailable;
}
OSStatus deny_update(CFDictionaryRef,CFDictionaryRef) { return errSecNotAvailable; }
OSStatus deny_delete(CFDictionaryRef) { return errSecNotAvailable; }
struct Pair {const void* replacement; const void* original;};
__attribute__((used,section("__DATA,__interpose,interposing")))
static const Pair interposes[]={
  {reinterpret_cast<const void*>(scoped_copy),reinterpret_cast<const void*>(SecItemCopyMatching)},
  {reinterpret_cast<const void*>(deny_add),reinterpret_cast<const void*>(SecItemAdd)},
  {reinterpret_cast<const void*>(deny_update),reinterpret_cast<const void*>(SecItemUpdate)},
  {reinterpret_cast<const void*>(deny_delete),reinterpret_cast<const void*>(SecItemDelete)},
};
bool routing_valid(const void* const* imports) {
  if (!imports) return false;
  for (size_t n=0;n<4;++n) if (imports[n]!=interposes[n].replacement) return false;
  return true;
}
template<class T> T security_spi(const char* name) {
  void* function=dlsym(RTLD_NEXT,name); Dl_info information{};
  if (!function || !dladdr(function,&information) || !information.dli_fname ||
      strcmp(information.dli_fname,"/System/Library/Frameworks/Security.framework/Versions/A/Security")!=0)
    return nullptr;
  return reinterpret_cast<T>(function);
}
int32_t failure(State& owner) { owner.ready=false; return owner.directory>=0 ? 3 : 1; }
bool capture_code(State& owner) {
  Ref<SecCodeRef> code; Ref<SecRequirementRef> requirement;
  Ref<SecStaticCodeRef> static_code; Ref<CFDictionaryRef> information;
  owner.phase=COLOSSUS_MAC_PROFILE_PHASE_CODE_SELF;
  if (SecCodeCopySelf(kSecCSDefaultFlags,code.out())!=errSecSuccess) return false;
  owner.phase=COLOSSUS_MAC_PROFILE_PHASE_CODE_STATIC;
  if (SecCodeCopyStaticCode(code.value,kSecCSDefaultFlags,static_code.out())!=errSecSuccess) return false;
  owner.phase=COLOSSUS_MAC_PROFILE_PHASE_CODE_DESIGNATED_REQUIREMENT;
  if (SecCodeCopyDesignatedRequirement(static_code.value,kSecCSDefaultFlags,requirement.out())!=errSecSuccess) return false;
  owner.phase=COLOSSUS_MAC_PROFILE_PHASE_CODE_DESIGNATED_VALIDITY;
  if (SecCodeCheckValidity(code.value,kSecCSDefaultFlags,requirement.value)!=errSecSuccess) return false;
  owner.phase=COLOSSUS_MAC_PROFILE_PHASE_CODE_SIGNING_INFORMATION;
  if (SecCodeCopySigningInformation(static_code.value,kSecCSSigningInformation,information.out())!=errSecSuccess) return false;
  CFTypeRef flags=CFDictionaryGetValue(information.value,kSecCodeInfoFlags);
  CFTypeRef hash=CFDictionaryGetValue(information.value,kSecCodeInfoUnique);
  uint32_t bits=0;
  owner.phase=COLOSSUS_MAC_PROFILE_PHASE_CODE_SIGNATURE_PROPERTIES;
  if (!flags || CFGetTypeID(flags)!=CFNumberGetTypeID() ||
      !CFNumberGetValue(static_cast<CFNumberRef>(flags),kCFNumberSInt32Type,&bits) ||
      !(bits & kSecCodeSignatureAdhoc) || !hash || CFGetTypeID(hash)!=CFDataGetTypeID() ||
      CFDataGetLength(static_cast<CFDataRef>(hash))!=20) return false;
  const UInt8* bytes=CFDataGetBytePtr(static_cast<CFDataRef>(hash));
  constexpr char hex[]="0123456789abcdef";
  std::string expression="cdhash H\"";
  for (size_t n=0;n<20;++n) {expression+=hex[bytes[n]>>4];expression+=hex[bytes[n]&15];}
  expression+='"';
  Ref<CFStringRef> text;
  text.value=CFStringCreateWithCString(kCFAllocatorDefault,expression.c_str(),kCFStringEncodingASCII);
  Ref<SecRequirementRef> exact;
  owner.phase=COLOSSUS_MAC_PROFILE_PHASE_CODE_EXACT_REQUIREMENT;
  if (!text.value || SecRequirementCreateWithString(text.value,kSecCSDefaultFlags,exact.out())!=errSecSuccess) return false;
  owner.phase=COLOSSUS_MAC_PROFILE_PHASE_CODE_EXACT_VALIDITY;
  if (SecCodeCheckValidity(code.value,kSecCSDefaultFlags,exact.value)!=errSecSuccess) return false;
  owner.code=code.release(); owner.requirement=exact.release();
  owner.code_hash=static_cast<CFDataRef>(CFRetain(hash));
  return true;
}
bool acl_valid_for_host(State& owner) {
  if (!owner.item || !owner.requirement || !owner.trusted_requirement) return false;
  Ref<SecAccessRef> access; Ref<CFArrayRef> acls; Ref<CFDataRef> expected;
  if (SecKeychainItemCopyAccess(owner.item,access.out())!=errSecSuccess || !access.value ||
      SecAccessCopyACLList(access.value,acls.out())!=errSecSuccess || !acls.value ||
      CFArrayGetCount(acls.value)>32 ||
      SecRequirementCopyData(owner.requirement,kSecCSDefaultFlags,expected.out())!=errSecSuccess) return false;
  bool has_read=false;
  for (CFIndex n=0;n<CFArrayGetCount(acls.value);++n) {
    SecACLRef acl=(SecACLRef)CFArrayGetValueAtIndex(acls.value,n);
    Ref<CFArrayRef> authorizations; authorizations.value=SecACLCopyAuthorizations(acl);
    if (!authorizations.value) return false;
    bool read=CFArrayContainsValue(authorizations.value,CFRangeMake(0,CFArrayGetCount(authorizations.value)),kSecACLAuthorizationAny) ||
        CFArrayContainsValue(authorizations.value,CFRangeMake(0,CFArrayGetCount(authorizations.value)),kSecACLAuthorizationDecrypt) ||
        CFArrayContainsValue(authorizations.value,CFRangeMake(0,CFArrayGetCount(authorizations.value)),kSecACLAuthorizationKeychainItemRead);
    if (!read) continue;
    has_read=true;
    Ref<CFArrayRef> applications; Ref<CFStringRef> description; SecKeychainPromptSelector prompt=0;
    if (SecACLCopyContents(acl,applications.out(),description.out(),&prompt)!=errSecSuccess ||
        !applications.value || CFArrayGetCount(applications.value)!=1) return false;
    CFTypeRef application=CFArrayGetValueAtIndex(applications.value,0);
    if (!application || CFGetTypeID(application)!=SecTrustedApplicationGetTypeID()) return false;
    Ref<SecRequirementRef> requirement; Ref<CFDataRef> actual;
    if (owner.trusted_requirement((SecTrustedApplicationRef)application,requirement.out())!=errSecSuccess || !requirement.value ||
        SecRequirementCopyData(requirement.value,kSecCSDefaultFlags,actual.out())!=errSecSuccess ||
        !CFEqual(expected.value,actual.value)) return false;
  }
  return has_read;
}
bool bind_store_file(State& owner) {
  char path[PATH_MAX]; UInt32 length=sizeof(path);
  if (!owner.keychain || SecKeychainGetPath(owner.keychain,&length,path)!=errSecSuccess || length>=sizeof(path)) return false;
  owner.store_path=std::string(path,length);
  const std::string expected=owner.directory_path+"/"+kStore;
  if (owner.store_path!=expected && owner.store_path!=expected+"-db") return false;
  owner.store_name=owner.store_path.substr(owner.directory_path.size()+1);
  owner.file=openat(owner.directory,owner.store_name.c_str(),O_RDONLY|O_NOFOLLOW|O_CLOEXEC);
  struct stat value{};
  if (owner.file<0 || fstat(owner.file,&value)!=0 || !private_metadata(value,false) || !no_extended_acl(owner.file)) return false;
  owner.file_id=identity(value); owner.store_recorded=true;
  // Apple's local file provider derives this exact advisory-lock basename from
  // the store basename. Recognizing it alone grants no creation authority: bind
  // only after successful creation in our previously empty retained namespace.
  // No later store or auxiliary inode generation is ever adopted here.
  std::array<uint8_t,CC_SHA1_DIGEST_LENGTH> digest{};
  if (!CC_SHA1(owner.store_name.data(),owner.store_name.size(),digest.data())) return false;
  char lock_name[12];
  snprintf(lock_name,sizeof(lock_name),".fl%02X%02X%02X%02X",digest[0],digest[1],digest[2],digest[3]);
  owner.aux_name=lock_name;
  owner.aux=openat(owner.directory,owner.aux_name.c_str(),O_RDONLY|O_NOFOLLOW|O_CLOEXEC);
  if (owner.aux<0 || fstat(owner.aux,&value)!=0 || !private_metadata(value,false) ||
      value.st_size!=0 || !no_extended_acl(owner.aux)) return false;
  owner.aux_id=identity(value); owner.aux_recorded=true;
  return store_valid(owner) && exact_entries(owner,true);
}
}

static int32_t prepare(const char* parent,const void* const imports[4],bool store_fixture_only) {
  State& owner=state(); std::lock_guard lock(owner.mutex); OwnedCall call;
  owner.phase=COLOSSUS_MAC_PROFILE_PHASE_ROUTING;
  if (!routing_valid(imports)) return 2;
  owner.phase=COLOSSUS_MAC_PROFILE_PHASE_PREPARE_GUARD;
  if (owner.attempted || owner.cleaned || owner.cleanup!=Cleanup::Live || !parent || strnlen(parent,PATH_MAX)>=PATH_MAX) return 1;
  owner.attempted=true; owner.routing=true;
  // Dedicated creator only; retain a restrictive process mask before ANY
  // Security call and never restore it. Launchers should also set it pre-exec.
  umask(0077);
  owner.phase=COLOSSUS_MAC_PROFILE_PHASE_SECURITY_SYMBOLS;
  if (!store_fixture_only) {
  owner.create_new=security_spi<CreateNew>("SecKeychainItemCreateNew");
  owner.set_attribute=security_spi<SetAttribute>("SecKeychainItemSetAttribute");
  owner.add_no_ui=security_spi<AddNoUI>("SecKeychainItemAddNoUI");
  owner.trusted_from=security_spi<TrustedFromRequirement>("SecTrustedApplicationCreateFromRequirement");
  owner.trusted_requirement=security_spi<TrustedCopyRequirement>("SecTrustedApplicationCopyRequirement");
  if (!owner.create_new || !owner.set_attribute || !owner.add_no_ui || !owner.trusted_from || !owner.trusted_requirement) return 2;
  }
  owner.parent_path=parent;
  owner.phase=COLOSSUS_MAC_PROFILE_PHASE_PARENT_PATH;
  if (owner.parent_path.empty() || owner.parent_path.front()!='/' || forbidden_name(owner.parent_path) || !canonical(owner.parent_path)) return 1;
  owner.parent=open(parent,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
  struct stat value{};
  owner.phase=COLOSSUS_MAC_PROFILE_PHASE_PARENT_OPEN;
  if (owner.parent<0) return 1;
  owner.phase=COLOSSUS_MAC_PROFILE_PHASE_PARENT_SECURITY;
  if (owner.parent<0 || fstat(owner.parent,&value)!=0 || !private_metadata(value,true) || !no_extended_acl(owner.parent)) return 1;
  owner.parent_id=identity(value);
  // Dedicated child only: this changes process-local interaction policy, never
  // a default, preference search list or trust record. Never restore UI later.
  owner.phase=COLOSSUS_MAC_PROFILE_PHASE_INTERACTION_POLICY;
  if (SecKeychainSetUserInteractionAllowed(false)!=errSecSuccess || !capture_code(owner)) return 1;
  owner.phase=COLOSSUS_MAC_PROFILE_PHASE_OWNED_DIRECTORY;
  if (mkdirat(owner.parent,kDirectory,0700)!=0) return 1;
  owner.directory_created=true;
  if (fstatat(owner.parent,kDirectory,&value,AT_SYMLINK_NOFOLLOW)!=0 || !private_metadata(value,true)) return 3;
  owner.directory_id=identity(value); owner.directory_path=owner.parent_path+"/"+kDirectory;
  owner.directory=openat(owner.parent,kDirectory,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
  if (owner.directory<0 || !namespace_valid(owner) || !exact_entries(owner,false)) return 3;
  // SecKeychainCreate requires canonical UTF-8, not arbitrary random bytes.
  // Encode 256-bit random entropy as ASCII hex; wipe both representations.
  std::array<uint8_t,32> password_entropy{}, secret{};
  std::array<uint8_t,64> password{};
  WipeOnExit entropy_wipe{password_entropy.data(),password_entropy.size()};
  WipeOnExit password_wipe{password.data(),password.size()};
  WipeOnExit secret_wipe{secret.data(),secret.size()};
  if (SecRandomCopyBytes(kSecRandomDefault,password_entropy.size(),password_entropy.data())!=errSecSuccess ||
      SecRandomCopyBytes(kSecRandomDefault,secret.size(),secret.data())!=errSecSuccess) {
    wipe(password_entropy.data(),password_entropy.size()); wipe(password.data(),password.size());
    wipe(secret.data(),secret.size()); return failure(owner);
  }
  constexpr char password_hex[]="0123456789abcdef";
  for (size_t n=0;n<password_entropy.size();++n) {
    password[n*2]=password_hex[password_entropy[n]>>4];
    password[n*2+1]=password_hex[password_entropy[n]&15];
  }
  wipe(password_entropy.data(),password_entropy.size());
  std::string path=owner.directory_path+"/"+kStore;
  // Record uncertain output BEFORE creation. No cleanup may adopt filesystem
  // entries after a failed call or a failed exact returned-inode binding.
  owner.creation_unknown=true;
  owner.phase=COLOSSUS_MAC_PROFILE_PHASE_OWNED_STORE;
  OSStatus created=SecKeychainCreate(path.c_str(),password.size(),password.data(),false,nullptr,&owner.keychain);
  wipe(password.data(),password.size());
  if (created!=errSecSuccess || !bind_store_file(owner)) {
    wipe(secret.data(),secret.size()); return failure(owner);
  }
  owner.creation_unknown=false;
  if (store_fixture_only) {
    // This does not enable scoped secret reads or ordinary host validation.
    owner.fixture_store_ready=true;
    return 0;
  }
  owner.phase=COLOSSUS_MAC_PROFILE_PHASE_OWNED_ITEM;
  // The public generic-password add APIs can choose a default Keychain after a
  // missing-store error. These weak legacy SPIs have no such fallback with a
  // nonnull genuine KeychainRef. Their absence is an unavailable development API.
  OSStatus item_created=owner.create_new(kSecGenericPasswordItemClass,0,secret.size(),secret.data(),&owner.item);
  wipe(secret.data(),secret.size());
  if (item_created!=errSecSuccess || !owner.item) return failure(owner);
  for (auto pair : {std::pair<SecKeychainAttrType,const char*>(kSecServiceItemAttr,kService),
                    std::pair<SecKeychainAttrType,const char*>(kSecAccountItemAttr,kAccount)}) {
    SecKeychainAttribute attribute{pair.first,static_cast<UInt32>(strlen(pair.second)),const_cast<char*>(pair.second)};
    if (owner.set_attribute(owner.item,&attribute)!=errSecSuccess) return failure(owner);
  }
  if (!store_valid(owner) || !code_valid(owner) || !owner.keychain ||
      owner.add_no_ui(owner.keychain,owner.item)!=errSecSuccess || !store_valid(owner) || !same_item_store(owner)) return failure(owner);
  Ref<SecTrustedApplicationRef> application; Ref<CFArrayRef> trusted; Ref<SecAccessRef> access;
  owner.phase=COLOSSUS_MAC_PROFILE_PHASE_OWNED_ITEM_ACCESS;
  if (owner.trusted_from("Colossus owned profile crypto",owner.requirement,application.out())!=errSecSuccess) return failure(owner);
  const void* applications[]={application.value};
  trusted.value=CFArrayCreate(kCFAllocatorDefault,applications,1,&kCFTypeArrayCallBacks);
  if (!trusted.value || SecAccessCreate(CFSTR("Colossus owned profile crypto"),trusted.value,access.out())!=errSecSuccess ||
      SecKeychainItemSetAccess(owner.item,access.value)!=errSecSuccess || !store_valid(owner) || !code_valid(owner) || !acl_valid_for_host(owner)) return failure(owner);
  owner.ready=true;
  owner.phase=COLOSSUS_MAC_PROFILE_PHASE_READY;
  return 0;
}

extern "C" int32_t colossus_mac_profile_crypto_prepare(const char* parent,const void* const imports[4]) {
  return prepare(parent,imports,false);
}
extern "C" int32_t colossus_mac_profile_crypto_prepare_store_fixture(const char* parent,const void* const imports[4]) {
  return prepare(parent,imports,true);
}
extern "C" int32_t colossus_mac_profile_crypto_store_fixture_valid() {
  State& owner=state(); std::lock_guard lock(owner.mutex); OwnedCall call;
  if (!owner.fixture_store_ready || owner.ready || owner.item || owner.cleaned) return 1;
  if (!code_valid(owner) || !store_valid(owner) || !owner.aux_recorded || !aux_bound(owner)) {
    owner.fixture_store_ready=false;owner.changed=true;return 3;
  }
  return 0;
}

extern "C" int32_t colossus_mac_profile_crypto_prepare_phase() {
  State& owner=state(); std::lock_guard lock(owner.mutex);
  return owner.phase;
}

extern "C" int32_t colossus_mac_profile_crypto_valid() {
  State& owner=state(); std::lock_guard lock(owner.mutex); OwnedCall call;
  if (!owner.ready || owner.cleaned) return owner.changed ? 3 : 1;
  if (!owner.routing || !code_valid(owner) || !store_bound(owner) || !same_item_store(owner) || !acl_valid_for_host(owner)) {
    owner.ready=false; owner.changed=true; return 3;
  }
  if (!store_valid(owner)) {owner.ready=false; return 1;}
  return 0;
}

extern "C" int32_t colossus_mac_profile_crypto_finish() {
  State& owner=state(); std::lock_guard lock(owner.mutex); OwnedCall call;
  // Holding this mutex also drains every in-flight read before store retirement.
  owner.ready=false;
  owner.fixture_store_ready=false;
  if (owner.cleanup==Cleanup::Finished || owner.cleaned) return 0;
  if (owner.creation_unknown || owner.changed) return 3;
  if (owner.directory<0) {
    if (owner.directory_created) return 3;
    if (owner.parent>=0) {close(owner.parent);owner.parent=-1;}
    owner.cleaned=true;owner.cleanup=Cleanup::Finished;return 0;
  }
  if (owner.cleanup<Cleanup::DirectoryRemoved && !namespace_valid(owner)) return 3;
  if (owner.cleanup==Cleanup::Live) {
    if (owner.store_recorded) {
      if (!owner.keychain || !file_bound(owner) || (owner.aux_recorded && !aux_bound(owner)) || !exact_entries(owner,true)) return 3;
      // A locked owned store is still an owned object; never unlock or prompt.
      if (SecKeychainLock(owner.keychain)!=errSecSuccess || !file_bound(owner) ||
          (owner.aux_recorded && !aux_bound(owner))) return 3;
    } else if (owner.keychain || owner.item || owner.file>=0 || !exact_entries(owner,false)) {
      // Never call a store API without its positively retained inode binding.
      return 3;
    }
    owner.cleanup=Cleanup::Locked;
  }
  if (owner.cleanup==Cleanup::Locked) {
    if (!namespace_valid(owner) || (owner.store_recorded && !file_bound(owner)) ||
        (owner.aux_recorded && !aux_bound(owner))) return 3;
    if (owner.item) {CFRelease(owner.item);owner.item=nullptr;}
    if (owner.keychain) {CFRelease(owner.keychain);owner.keychain=nullptr;}
    owner.cleanup=Cleanup::Released;
  }
  if (owner.cleanup==Cleanup::Released) {
    if (!namespace_valid(owner) || !exact_entries(owner,owner.store_recorded) ||
        (owner.store_recorded && !file_bound(owner)) ||
        (owner.aux_recorded && !aux_bound(owner))) return 3;
    if (owner.retired_name.empty()) {
      std::array<uint8_t,16> nonce{};
      if (SecRandomCopyBytes(kSecRandomDefault,nonce.size(),nonce.data())!=errSecSuccess) return 3;
      constexpr char hex[]="0123456789abcdef";
      std::string name=".retired-profile-crypto-";
      for (uint8_t byte : nonce) {name+=hex[byte>>4];name+=hex[byte&15];}
      owner.retired_name=name; // Retain the proposed namespace before mutation.
    }
    if (renameatx_np(owner.parent,kDirectory,owner.parent,owner.retired_name.c_str(),RENAME_EXCL)!=0) return 3;
    owner.cleanup=Cleanup::Retired;
    if (!namespace_valid(owner)) {owner.changed=true;return 3;}
  }
  if (owner.cleanup==Cleanup::Retired) {
    if (!namespace_valid(owner) || !exact_entries(owner,owner.store_recorded) ||
        (owner.aux_recorded && !aux_bound(owner))) return 3;
    if (owner.store_recorded) {
      if (!file_bound(owner)) {owner.changed=true;return 3;}
      if (unlinkat(owner.directory,owner.store_name.c_str(),0)!=0) return 3;
    }
    owner.cleanup=Cleanup::FileRemoved;
  }
  if (owner.cleanup==Cleanup::FileRemoved) {
    if (!namespace_valid(owner) || !exact_entries(owner,false)) return 3;
    if (owner.file>=0) {close(owner.file);owner.file=-1;}
    if (owner.aux_recorded) {
      if (!aux_bound(owner) || unlinkat(owner.directory,owner.aux_name.c_str(),0)!=0) return 3;
    }
    owner.cleanup=Cleanup::AuxRemoved;
  }
  if (owner.cleanup==Cleanup::AuxRemoved) {
    if (!namespace_valid(owner) || !exact_entries(owner,false)) return 3;
    if (owner.aux>=0) {close(owner.aux);owner.aux=-1;}
    if (unlinkat(owner.parent,owner.retired_name.c_str(),AT_REMOVEDIR)!=0) return 3;
    owner.cleanup=Cleanup::DirectoryRemoved;
  }
  if (owner.cleanup==Cleanup::DirectoryRemoved) {
    struct stat held{};
    if (!parent_valid(owner) || owner.directory<0 || fstat(owner.directory,&held)!=0 ||
        !owner.directory_id.same(held) || !private_metadata(held,true) || !no_extended_acl(owner.directory)) return 3;
    struct stat entry{};
    if (fstatat(owner.parent,owner.retired_name.c_str(),&entry,AT_SYMLINK_NOFOLLOW)==0 || errno!=ENOENT) return 3;
    // A failed flush is retryable after verified removal, without touching a
    // pathname replacement or requiring a released/unlocked Keychain handle.
    if (fsync(owner.parent)!=0) return 3;
    close(owner.directory);owner.directory=-1;close(owner.parent);owner.parent=-1;
    if (owner.code) {CFRelease(owner.code);owner.code=nullptr;}
    if (owner.requirement) {CFRelease(owner.requirement);owner.requirement=nullptr;}
    if (owner.code_hash) {CFRelease(owner.code_hash);owner.code_hash=nullptr;}
    owner.cleaned=true;owner.cleanup=Cleanup::Finished;
  }
  return owner.cleanup==Cleanup::Finished ? 0 : 3;
}
#pragma clang diagnostic pop
