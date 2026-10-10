// REVIEW BEFORE EXECUTION. Real owned Keychain creation/removal is exercised.
// No Chromium, personal item query, trust mutation, or credential output.
#include "profile_crypto.h"
#import <Security/Security.h>
#include <sys/stat.h>
#include <unistd.h>
#include <limits.h>
#include <cstdio>
#include <string>
#include <vector>
#include <sstream>
#include <cerrno>
#include <cstdlib>
#include <dirent.h>
#include <fcntl.h>
#include <sys/acl.h>
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"

static bool private_capture(int descriptor) {
  struct stat value{};const int flags=fcntl(descriptor,F_GETFL);
  if(flags<0 || (flags&O_ACCMODE)==O_RDONLY || fstat(descriptor,&value)!=0 ||
      !S_ISREG(value.st_mode) || value.st_uid!=geteuid() || value.st_nlink!=1 ||
      (value.st_mode&0777)!=0600)return false;
  filesec_t security=filesec_init();if(!security)return false;
  int present=0;bool empty=false;
  if(fstatx_np(descriptor,&value,security)==0 &&
      filesec_query_property(security,FILESEC_ACL,&present)==0) {
    if(!present)empty=true;
    else {
      acl_t acl=nullptr;
      if(filesec_get_property(security,FILESEC_ACL,&acl)==0 && acl) {
        acl_entry_t entry{};errno=0;
        const int result=acl_get_entry(acl,ACL_FIRST_ENTRY,&entry),error=errno;
        empty=acl_valid(acl)==0 && result==-1 && error==EINVAL;acl_free(acl);
      }
    }
  }
  filesec_free(security);return empty;
}

struct Metadata {
  OSStatus default_status{},search_status{};
  std::string default_path;
  std::vector<std::string> search_paths;
  bool operator==(const Metadata&) const=default;
};
static void stage(const char* value) {
  // The reviewed runner captures this stream in an existing private mode0600
  // file. Flush before Security calls so a hang leaves a bounded last stage.
  fprintf(stderr,"stage=%s\n",value);fflush(stderr);
}
static bool requirement_parser_proof() {
  // Public parser-only proof, before store operations. The fixed zero hash is
  // never matched to a process; this tests the exact requirement grammar.
  SecRequirementRef valid=nullptr,invalid=nullptr,roundtrip=nullptr;
  CFDataRef encoded=nullptr;
  const OSStatus accepted=SecRequirementCreateWithString(
      CFSTR("cdhash H\"0000000000000000000000000000000000000000\""),kSecCSDefaultFlags,&valid);
  const OSStatus rejected=SecRequirementCreateWithString(
      CFSTR("cdhash 0000000000000000000000000000000000000000"),kSecCSDefaultFlags,&invalid);
  const bool good=accepted==errSecSuccess && valid && rejected!=errSecSuccess && !invalid &&
      SecRequirementCopyData(valid,kSecCSDefaultFlags,&encoded)==errSecSuccess && encoded &&
      SecRequirementCreateWithData(encoded,kSecCSDefaultFlags,&roundtrip)==errSecSuccess && roundtrip;
  if(roundtrip)CFRelease(roundtrip);
  if(encoded)CFRelease(encoded);
  if(invalid)CFRelease(invalid);
  if(valid)CFRelease(valid);
  return good;
}
static const char* phase_name(int phase) {
  switch(phase) {
    case COLOSSUS_MAC_PROFILE_PHASE_NOT_ATTEMPTED:return "not_attempted";
    case COLOSSUS_MAC_PROFILE_PHASE_ROUTING:return "routing";
    case COLOSSUS_MAC_PROFILE_PHASE_PREPARE_GUARD:return "prepare_guard";
    case COLOSSUS_MAC_PROFILE_PHASE_SECURITY_SYMBOLS:return "security_symbols";
    case COLOSSUS_MAC_PROFILE_PHASE_PARENT_PATH:return "parent_path";
    case COLOSSUS_MAC_PROFILE_PHASE_PARENT_OPEN:return "parent_open";
    case COLOSSUS_MAC_PROFILE_PHASE_PARENT_SECURITY:return "parent_security";
    case COLOSSUS_MAC_PROFILE_PHASE_INTERACTION_POLICY:return "interaction_policy";
    case COLOSSUS_MAC_PROFILE_PHASE_CODE_SELF:return "code_self";
    case COLOSSUS_MAC_PROFILE_PHASE_CODE_STATIC:return "code_static";
    case COLOSSUS_MAC_PROFILE_PHASE_CODE_DESIGNATED_REQUIREMENT:return "code_designated_requirement";
    case COLOSSUS_MAC_PROFILE_PHASE_CODE_DESIGNATED_VALIDITY:return "code_designated_validity";
    case COLOSSUS_MAC_PROFILE_PHASE_CODE_SIGNING_INFORMATION:return "code_signing_information";
    case COLOSSUS_MAC_PROFILE_PHASE_CODE_SIGNATURE_PROPERTIES:return "code_signature_properties";
    case COLOSSUS_MAC_PROFILE_PHASE_CODE_EXACT_REQUIREMENT:return "code_exact_requirement";
    case COLOSSUS_MAC_PROFILE_PHASE_CODE_EXACT_VALIDITY:return "code_exact_validity";
    case COLOSSUS_MAC_PROFILE_PHASE_OWNED_DIRECTORY:return "owned_directory";
    case COLOSSUS_MAC_PROFILE_PHASE_OWNED_STORE:return "owned_store";
    case COLOSSUS_MAC_PROFILE_PHASE_OWNED_ITEM:return "owned_item";
    case COLOSSUS_MAC_PROFILE_PHASE_OWNED_ITEM_ACCESS:return "owned_item_access";
    case COLOSSUS_MAC_PROFILE_PHASE_READY:return "ready";
    default:return "invalid_phase";
  }
}
static bool keychain_path(SecKeychainRef value,std::string* output) {
  char bytes[PATH_MAX];UInt32 size=sizeof(bytes);
  if (!value || SecKeychainGetPath(value,&size,bytes)!=errSecSuccess || size>=sizeof(bytes)) return false;
  *output=std::string(bytes,size);return true;
}
static bool snapshot(Metadata* output) {
  SecKeychainRef default_keychain=nullptr;
  output->default_status=SecKeychainCopyDefault(&default_keychain);
  if (output->default_status==errSecSuccess && !keychain_path(default_keychain,&output->default_path)) return false;
  if (default_keychain) CFRelease(default_keychain);
  CFArrayRef search=nullptr;output->search_status=SecKeychainCopySearchList(&search);
  if (output->search_status!=errSecSuccess || !search || CFArrayGetCount(search)>64) {
    if(search)CFRelease(search);return false;
  }
  for (CFIndex n=0;n<CFArrayGetCount(search);++n) {
    std::string path;
    if (!keychain_path((SecKeychainRef)CFArrayGetValueAtIndex(search,n),&path)) {CFRelease(search);return false;}
    output->search_paths.push_back(path);
  }
  CFRelease(search);return true;
}
static std::string json_string(const std::string& text) {
  std::ostringstream result;result<<'"';
  for(unsigned char ch:text) {
    if(ch=='"'||ch=='\\')result<<'\\'<<char(ch);
    else if(ch<32) {char escaped[7];snprintf(escaped,sizeof(escaped),"\\u%04x",ch);result<<escaped;}
    else result<<char(ch);
  }
  result<<'"';return result.str();
}
static void print_metadata(const Metadata& value) {
  printf("{\"default_status\":%d,\"default_path\":%s,\"search_status\":%d,\"search_paths\":[",
      value.default_status,json_string(value.default_path).c_str(),value.search_status);
  for(size_t n=0;n<value.search_paths.size();++n)printf("%s%s",n?",":"",json_string(value.search_paths[n]).c_str());
  printf("]}");
}
static CFDictionaryRef query(CFStringRef service) {
  const void* keys[]={kSecClass,kSecAttrService,kSecAttrAccount,kSecMatchLimit,kSecReturnData,kSecReturnAttributes};
  const void* values[]={kSecClassGenericPassword,service,CFSTR("Chromium"),kSecMatchLimitOne,kCFBooleanTrue,kCFBooleanTrue};
  return CFDictionaryCreate(kCFAllocatorDefault,keys,values,6,&kCFTypeDictionaryKeyCallBacks,&kCFTypeDictionaryValueCallBacks);
}
int main(int argc,char** argv) {
  umask(0077);
  alarm(20);
  const bool store_only=argc==3 && std::string(argv[2])=="--store-only";
  if(argc!=2 && !store_only)return 10;
  // Validate the actual inherited writable regular-file captures, including ACL
  // absence, before any personal metadata snapshot. A terminal/pipe is denied.
  if(!private_capture(STDOUT_FILENO)||!private_capture(STDERR_FILENO))return 17;
  char parent[PATH_MAX];
  if(!realpath(argv[1],parent)||std::string(parent)!=argv[1])return 11;
  const void* imports[]={reinterpret_cast<const void*>(SecItemCopyMatching),reinterpret_cast<const void*>(SecItemAdd),
                        reinterpret_cast<const void*>(SecItemUpdate),reinterpret_cast<const void*>(SecItemDelete)};
  // Null parent must return Denied after import validation, with no store APIs.
  const int routing=colossus_mac_profile_crypto_prepare(nullptr,imports);
  if(routing!=1) {printf("{\"production_acceptance\":false,\"routing_status\":%d,\"store_operations_started\":false}\n",routing);return 12;}
  stage("disable_process_local_interaction");
  if(SecKeychainSetUserInteractionAllowed(false)!=errSecSuccess)return 15;
  stage("requirement_parser_proof");
  const bool parser_proved=requirement_parser_proof();
  if(!parser_proved) {
    printf("{\"production_acceptance\":false,\"requirement_parser_proof_passed\":false,\"store_operations_started\":false}\n");
    return 16;
  }
  // Invoke the exact imported addresses checked by prepare. A separate direct
  // symbol call could have a different binding from the reviewed address.
  using Copy=OSStatus(*)(CFDictionaryRef,CFTypeRef*);
  using Add=OSStatus(*)(CFDictionaryRef,CFTypeRef*);
  using Update=OSStatus(*)(CFDictionaryRef,CFDictionaryRef);
  using Delete=OSStatus(*)(CFDictionaryRef);
  const Copy checked_copy=reinterpret_cast<Copy>(const_cast<void*>(imports[0]));
  const Add checked_add=reinterpret_cast<Add>(const_cast<void*>(imports[1]));
  const Update checked_update=reinterpret_cast<Update>(const_cast<void*>(imports[2]));
  const Delete checked_delete=reinterpret_cast<Delete>(const_cast<void*>(imports[3]));
  Metadata before,after;
  stage("metadata_before");
  if(!snapshot(&before))return 13;
  stage("prepare_owned_store");
  const int prepared=store_only ? colossus_mac_profile_crypto_prepare_store_fixture(parent,imports) :
      colossus_mac_profile_crypto_prepare(parent,imports);
  const int phase=colossus_mac_profile_crypto_prepare_phase();
  fprintf(stderr,"prepare_phase=%s\n",phase_name(phase));fflush(stderr);
  bool scoped=false,foreign_denied=false,mutations_denied=false;
  int valid=1,store_valid=1;
  if(prepared==0 && store_only) {
    store_valid=colossus_mac_profile_crypto_store_fixture_valid();
    // Creation/removal must never enable the ordinary profile secret capability.
    valid=colossus_mac_profile_crypto_valid();
  } else if(prepared==0) {
    valid=colossus_mac_profile_crypto_valid();
    CFDictionaryRef owned=query(CFSTR("Chromium Safe Storage"));
    CFDictionaryRef foreign=query(CFSTR("Colossus Foreign Fixture"));
    CFTypeRef result=nullptr;
    stage("checked_owned_query");
    const OSStatus status=checked_copy(owned,&result);
    if(status==errSecSuccess && result && CFGetTypeID(result)==CFDictionaryGetTypeID()) {
      CFTypeRef data=CFDictionaryGetValue((CFDictionaryRef)result,kSecValueData);
      // The actual random owned secret is never logged, hashed or serialized.
      scoped=data && CFGetTypeID(data)==CFDataGetTypeID() && CFDataGetLength((CFDataRef)data)==32;
    }
    if(result)CFRelease(result);result=nullptr;
    stage("checked_foreign_query");
    foreign_denied=checked_copy(foreign,&result)==errSecNotAvailable && !result;
    stage("checked_mutations_denied");
    mutations_denied=checked_add(owned,nullptr)==errSecNotAvailable &&
        checked_update(owned,owned)==errSecNotAvailable && checked_delete(owned)==errSecNotAvailable;
    if(result)CFRelease(result);
    CFRelease(owned);CFRelease(foreign);
  }
  stage("finish_owned_store");
  const int finished=colossus_mac_profile_crypto_finish();
  const int finished_again=colossus_mac_profile_crypto_finish();
  stage("metadata_after");
  const bool after_ok=snapshot(&after);
  bool removed=true;DIR* directory=opendir(parent);
  if(!directory)return 14;
  while(dirent* entry=readdir(directory)) {
    if(std::string(entry->d_name)!="." && std::string(entry->d_name)!="..")removed=false;
  }
  closedir(directory);
  const bool unchanged=after_ok && before==after;
  printf("{\"production_acceptance\":false,\"certificate_pki_acceptance\":false,\"chromium_launched\":false,"
         "\"prepare_status\":%d,\"valid_status\":%d,\"owned_random_secret_length_verified\":%s,"
         "\"prepare_phase\":%d,\"requirement_parser_proof_passed\":%s,"
         "\"profile_store_lifecycle_only\":%s,"
         "\"store_fixture_valid_status\":%d,"
         "\"foreign_query_denied\":%s,\"mutation_queries_denied\":%s,\"finish_status\":%d,\"second_finish_status\":%d,"
         "\"owned_artifacts_removed\":%s,\"default_search_metadata_unchanged\":%s,\"before\":",
      prepared,valid,scoped?"true":"false",phase,parser_proved?"true":"false",store_only?"true":"false",store_valid,foreign_denied?"true":"false",mutations_denied?"true":"false",
      finished,finished_again,removed?"true":"false",unchanged?"true":"false");
  print_metadata(before);printf(",\"after\":");print_metadata(after);printf("}\n");
  if(fflush(stdout)!=0 || ferror(stdout))return 18;
  const bool scope_proved=store_only ? phase==COLOSSUS_MAC_PROFILE_PHASE_OWNED_STORE :
      phase==COLOSSUS_MAC_PROFILE_PHASE_READY && scoped && foreign_denied && mutations_denied;
  const bool validation_proved=store_only ? store_valid==0 && valid==COLOSSUS_MAC_PROFILE_CRYPTO_DENIED : valid==0;
  return parser_proved && prepared==0 && validation_proved && scope_proved &&
      finished==0 && finished_again==0 && removed && unchanged ? 0 : 1;
}
#pragma clang diagnostic pop
