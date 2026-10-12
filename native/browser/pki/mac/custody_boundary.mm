#include "custody_boundary.h"
#if !defined(COLOSSUS_MAC_OWNED_PKI_PROTOTYPE) || defined(NDEBUG)
#error "Custody authority is an explicitly admitted Mac Debug native boundary"
#endif
#import <Security/Security.h>
#include <CommonCrypto/CommonDigest.h>
#include <mach-o/dyld.h>
#include <sys/stat.h>
#include <sys/acl.h>
#include <fcntl.h>
#include <unistd.h>
#include <dirent.h>
#include <limits.h>
#include <array>
#include <cstring>
#include <mutex>
#include <string>
#include <optional>
#include <cstdio>
#include <cerrno>

namespace {
struct Identity {
  dev_t device{};ino_t inode{};uid_t uid{};
  static Identity From(const struct stat& s) { return {s.st_dev,s.st_ino,s.st_uid}; }
  bool Same(const struct stat& s) const { return device==s.st_dev&&inode==s.st_ino&&uid==s.st_uid; }
};
bool no_acl(int fd) {
  filesec_t security=filesec_init();if(!security)return false;
  struct stat metadata{};int present=0;bool empty=false;
  if(fstatx_np(fd,&metadata,security)==0&&filesec_query_property(security,FILESEC_ACL,&present)==0) {
    if(!present)empty=true;
    else {acl_t acl=nullptr;if(filesec_get_property(security,FILESEC_ACL,&acl)==0&&acl){acl_entry_t entry{};errno=0;empty=acl_valid(acl)==0&&acl_get_entry(acl,ACL_FIRST_ENTRY,&entry)==-1&&errno==EINVAL;acl_free(acl);}}
  }
  filesec_free(security);return empty;
}
bool metadata(int fd,bool directory,struct stat* s) {
  return fd>=0&&fstat(fd,s)==0&&s->st_uid==geteuid()&&
      (s->st_mode&07777)==(directory?0700:0600)&&
      (directory?S_ISDIR(s->st_mode):S_ISREG(s->st_mode)&&s->st_nlink==1)&&no_acl(fd);
}
bool canonical(const std::string& path) {
  char result[PATH_MAX];return !path.empty()&&path[0]=='/'&&path.size()<PATH_MAX&&
      path.find('\0')==std::string::npos&&realpath(path.c_str(),result)&&path==result;
}
bool descriptor_path(int fd,std::string* path) {
  char output[PATH_MAX];if(fcntl(fd,F_GETPATH,output)!=0)return false;*path=output;return canonical(*path);
}
struct Window {
  uint64_t sequence;uint32_t operation;int directory=-1;bool sealed=false;
  ~Window(){if(directory>=0)close(directory);}
};
struct Boundary {
  std::mutex mutex;std::string path;int parent=-1;Identity parent_id{},namespace_id{},store_id{},lock_id{};
  bool namespace_recorded=false,store_recorded=false,retirement_started=false,retire_only=false;std::string store_name,lock_name;
  int retirement_step=-1;std::string retired_name;
  std::array<uint8_t,32> generation{};uint64_t sequence=0;std::optional<Window> window;
  ~Boundary(){window.reset();if(parent>=0)close(parent);volatile uint8_t* p=generation.data();for(size_t i=0;i<generation.size();++i)p[i]=0;}
  bool Parent() const {
    struct stat held{},entry{};return metadata(parent,true,&held)&&parent_id.Same(held)&&
        lstat(path.c_str(),&entry)==0&&parent_id.Same(entry)&&canonical(path);
  }
  bool HeldDirectory(int fd) const {
    struct stat held{};return namespace_recorded&&metadata(fd,true,&held)&&namespace_id.Same(held);
  }
  bool CurrentDirectory(int fd) const {
    std::string current;return HeldDirectory(fd)&&descriptor_path(fd,&current)&&current==path+"/colossus-tls-identity";
  }
  bool CurrentStore(int fd) const {
    struct stat store{},lock{};
    return store_recorded&&fstatat(fd,store_name.c_str(),&store,AT_SYMLINK_NOFOLLOW)==0&&store_id.Same(store)&&
        fstatat(fd,lock_name.c_str(),&lock,AT_SYMLINK_NOFOLLOW)==0&&lock_id.Same(lock)&&
        S_ISREG(store.st_mode)&&S_ISREG(lock.st_mode)&&store.st_nlink==1&&lock.st_nlink==1&&lock.st_size==0;
  }
  bool Exclusive(uint64_t expected) const {
    return window&&window->sequence==expected&&Parent()&&
        (window->operation==3||HeldDirectory(window->directory));
  }
  bool EmptyDirectory(int fd) const {
    int copy=fcntl(fd,F_DUPFD_CLOEXEC,3);if(copy<0)return false;
    DIR* entries=fdopendir(copy);if(!entries){close(copy);return false;}bool valid=true;rewinddir(entries);
    while(valid){errno=0;const dirent* entry=readdir(entries);if(!entry){if(errno)valid=false;break;}if(strcmp(entry->d_name,".")&&strcmp(entry->d_name,".."))valid=false;}
    closedir(entries);return valid;
  }
  bool Outputs(int file,int lock,struct stat* store,struct stat* auxiliary,std::string* name,std::string* aux) const {
    std::string first,second;
    if(!metadata(file,false,store)||!metadata(lock,false,auxiliary)||store->st_size<=0||auxiliary->st_size!=0||
       !descriptor_path(file,&first)||!descriptor_path(lock,&second))return false;
    const std::string prefix=path+"/colossus-tls-identity/";
    if(first!=prefix+"colossus-tls-identity.keychain"&&first!=prefix+"colossus-tls-identity.keychain-db")return false;
    *name=first.substr(prefix.size());std::array<uint8_t,20> digest{};
    if(!CC_SHA1(name->data(),static_cast<CC_LONG>(name->size()),digest.data()))return false;
    char derived[12];snprintf(derived,sizeof(derived),".fl%02X%02X%02X%02X",digest[0],digest[1],digest[2],digest[3]);*aux=derived;
    if(second!=prefix+*aux)return false;
    struct stat first_entry{},second_entry{};
    if(fstatat(window->directory,name->c_str(),&first_entry,AT_SYMLINK_NOFOLLOW)||fstatat(window->directory,aux->c_str(),&second_entry,AT_SYMLINK_NOFOLLOW)||
       !Identity::From(*store).Same(first_entry)||!Identity::From(*auxiliary).Same(second_entry))return false;
    int copy=fcntl(window->directory,F_DUPFD_CLOEXEC,3);if(copy<0)return false;
    DIR* entries=fdopendir(copy);if(!entries){close(copy);return false;}unsigned seen=0;bool valid=true;rewinddir(entries);
    while(valid){errno=0;const dirent* entry=readdir(entries);if(!entry){if(errno)valid=false;break;}
      std::string current=entry->d_name;if(current=="."||current=="..")continue;
      unsigned bit=current==*name?1:current==*aux?2:0;if(!bit||(seen&bit))valid=false;seen|=bit;
    }
    closedir(entries);return valid&&seen==3;
  }
  bool RetirementEntries(uint32_t step) const {
    const int fd=window->directory;
    const auto absent=[fd](const std::string& name){struct stat s{};return fstatat(fd,name.c_str(),&s,AT_SYMLINK_NOFOLLOW)!=0&&errno==ENOENT;};
    if(store_recorded&&step>=1&&!absent(store_name))return false;
    if(store_recorded&&step>=2&&!absent(lock_name))return false;
    int copy=fcntl(fd,F_DUPFD_CLOEXEC,3);if(copy<0)return false;
    DIR* entries=fdopendir(copy);if(!entries){close(copy);return false;}unsigned seen=0;bool valid=true;rewinddir(entries);
    while(valid){errno=0;const dirent* entry=readdir(entries);if(!entry){if(errno)valid=false;break;}
      const std::string current=entry->d_name;if(current=="."||current=="..")continue;
      const unsigned bit=store_recorded&&step<1&&current==store_name?1:store_recorded&&step<2&&current==lock_name?2:0;
      struct stat s{};
      if(!bit||(seen&bit)||fstatat(fd,current.c_str(),&s,AT_SYMLINK_NOFOLLOW)||!(bit==1?store_id:lock_id).Same(s)||
          !S_ISREG(s.st_mode)||s.st_nlink!=1||(s.st_mode&07777)!=0600||(bit==2&&s.st_size!=0))valid=false;
      seen|=bit;
    }
    closedir(entries);return valid&&seen==(store_recorded?step==0?3:step==1?2:0:0);
  }
};
}

uint32_t colossus_custody_parent_pid(void){return getppid()>1?static_cast<uint32_t>(getppid()):0;}
bool colossus_custody_executable(char* output,size_t capacity) {
  if(!output||capacity<PATH_MAX)return false;
  char raw[PATH_MAX];uint32_t size=sizeof(raw);return _NSGetExecutablePath(raw,&size)==0&&realpath(raw,output);
}
bool colossus_custody_code(const char* path,const uint8_t* hash) {
  if(!path||!hash)return false;std::string expression="cdhash H\"";constexpr char hex[]="0123456789abcdef";
  for(size_t i=0;i<20;++i){expression+=hex[hash[i]>>4];expression+=hex[hash[i]&15];}expression+='"';
  CFURLRef url=CFURLCreateFromFileSystemRepresentation(kCFAllocatorDefault,reinterpret_cast<const UInt8*>(path),strlen(path),false);
  CFStringRef text=CFStringCreateWithCString(kCFAllocatorDefault,expression.c_str(),kCFStringEncodingASCII);
  SecStaticCodeRef code=nullptr;SecRequirementRef requirement=nullptr;
  const SecCSFlags flags=kSecCSStrictValidate|kSecCSCheckAllArchitectures|kSecCSCheckNestedCode|kSecCSNoNetworkAccess;
  bool valid=url&&text&&SecStaticCodeCreateWithPath(url,kSecCSDefaultFlags,&code)==errSecSuccess&&
      SecRequirementCreateWithString(text,kSecCSDefaultFlags,&requirement)==errSecSuccess&&
      SecStaticCodeCheckValidity(code,flags,requirement)==errSecSuccess;
  if(requirement)CFRelease(requirement);if(code)CFRelease(code);if(text)CFRelease(text);if(url)CFRelease(url);return valid;
}
void* colossus_custody_create(const char* path,const uint8_t* generation) {
  if(!path||!generation||getuid()==0||getuid()!=geteuid()||!canonical(path))return nullptr;
  bool nonzero=false;for(size_t i=0;i<32;++i)nonzero=nonzero||generation[i]!=0;if(!nonzero)return nullptr;
  auto* boundary=new Boundary();boundary->path=path;memcpy(boundary->generation.data(),generation,32);
  boundary->parent=open(path,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);struct stat metadata_value{};
  if(!metadata(boundary->parent,true,&metadata_value)){delete boundary;return nullptr;}
  boundary->parent_id=Identity::From(metadata_value);if(!boundary->Parent()){delete boundary;return nullptr;}return boundary;
}
int colossus_custody_parent_fd(void* raw){return raw?static_cast<Boundary*>(raw)->parent:-1;}
bool colossus_custody_begin(void* raw,const uint8_t* generation,int parent,int directory,uint64_t sequence,uint32_t operation) {
  if(!raw||!generation)return false;auto& owner=*static_cast<Boundary*>(raw);std::lock_guard guard(owner.mutex);
  struct stat parent_value{},directory_value{};
  if(owner.window||!owner.Parent()||sequence<=owner.sequence||operation>3||memcmp(owner.generation.data(),generation,32)||
     !metadata(parent,true,&parent_value)||!owner.parent_id.Same(parent_value))return false;
  owner.sequence=sequence;
  if(operation==3) {
    if(directory!=-1||owner.namespace_recorded)return false;
    struct stat entry{};if(fstatat(owner.parent,"colossus-tls-identity",&entry,AT_SYMLINK_NOFOLLOW)==0||errno!=ENOENT)return false;
  } else {
    if(!metadata(directory,true,&directory_value)||!owner.HeldDirectory(directory)||
       (operation!=2&&(owner.retirement_started||owner.retire_only))||(operation==0&&(owner.store_recorded||!owner.CurrentDirectory(directory)||!owner.EmptyDirectory(directory)))||
       (operation==1&&(!owner.CurrentDirectory(directory)||!owner.CurrentStore(directory)))||
       (operation==2&&!owner.retirement_started&&(!owner.CurrentDirectory(directory)||(owner.store_recorded?!owner.CurrentStore(directory):!owner.EmptyDirectory(directory)))))return false;
    if(operation==2)owner.retirement_started=true;
  }
  owner.window.emplace();owner.window->sequence=sequence;owner.window->operation=operation;
  if(operation!=3){owner.window->directory=fcntl(directory,F_DUPFD_CLOEXEC,3);if(owner.window->directory<0){owner.window.reset();return false;}}
  return owner.Exclusive(sequence);
}
bool colossus_custody_exclusive(void* raw,uint64_t sequence) {
  if(!raw)return false;auto& owner=*static_cast<Boundary*>(raw);std::lock_guard guard(owner.mutex);return owner.Exclusive(sequence);
}
bool colossus_custody_seal(void* raw,uint64_t sequence,const int* fds,size_t count) {
  if(!raw||!fds)return false;auto& owner=*static_cast<Boundary*>(raw);std::lock_guard guard(owner.mutex);
  if(!owner.Exclusive(sequence)||owner.window->sealed)return false;
  if(owner.window->operation==3) {
    struct stat output{},entry{};std::string path;
    if(count!=1||!metadata(fds[0],true,&output)||!descriptor_path(fds[0],&path)||path!=owner.path+"/colossus-tls-identity"||
       fstatat(owner.parent,"colossus-tls-identity",&entry,AT_SYMLINK_NOFOLLOW)||!Identity::From(output).Same(entry)||!owner.EmptyDirectory(fds[0]))return false;
    owner.namespace_id=Identity::From(output);owner.namespace_recorded=true;
  } else {
    struct stat store{},aux{};std::string name,lock;
    if(count!=2||(owner.window->operation!=0&&owner.window->operation!=1)||!owner.Outputs(fds[0],fds[1],&store,&aux,&name,&lock)||
       (owner.window->operation==1&&(name!=owner.store_name||lock!=owner.lock_name||!owner.lock_id.Same(aux))))return false;
    owner.store_id=Identity::From(store);owner.lock_id=Identity::From(aux);owner.store_name=name;owner.lock_name=lock;owner.store_recorded=true;
  }
  owner.window->sealed=true;return owner.Exclusive(sequence);
}
bool colossus_custody_checkpoint(void* raw,uint64_t sequence,uint32_t step) {
  if(!raw||step>3)return false;auto& owner=*static_cast<Boundary*>(raw);std::lock_guard guard(owner.mutex);
  if(!owner.Exclusive(sequence)||owner.window->operation!=2||static_cast<int>(step)>owner.retirement_step+1)return false;
  if(step<3){std::string path;if(!descriptor_path(owner.window->directory,&path))return false;
    const std::string prefix=owner.path+"/.retired-tls-identity-";
    if(path.size()!=prefix.size()+32||path.substr(0,prefix.size())!=prefix)return false;
    for(char c:path.substr(prefix.size()))if(!((c>='0'&&c<='9')||(c>='a'&&c<='f')))return false;
    const std::string name=path.substr(owner.path.size()+1);struct stat entry{};
    if((!owner.retired_name.empty()&&owner.retired_name!=name)||fstatat(owner.parent,name.c_str(),&entry,AT_SYMLINK_NOFOLLOW)||
       !owner.namespace_id.Same(entry)||!S_ISDIR(entry.st_mode)||!owner.RetirementEntries(step))return false;
    owner.retired_name=name;
  }else {struct stat held{},entry{};
    // macOS may retain a nonzero directory link count on an open descriptor
    // after successful rmdir. The trusted local owner checkpoints only after
    // positive exact unlinkat; retained identity, exact absent name, immutable
    // exclusion and parent fsync are the removal proof.
    if(owner.retired_name.empty()||fstat(owner.window->directory,&held)||!owner.namespace_id.Same(held)||
       fstatat(owner.parent,owner.retired_name.c_str(),&entry,AT_SYMLINK_NOFOLLOW)==0||errno!=ENOENT||fsync(owner.parent))return false;
  }
  if(static_cast<int>(step)>owner.retirement_step)owner.retirement_step=static_cast<int>(step);return true;
}
bool colossus_custody_retire_unchanged(void* raw,uint64_t sequence,const int* fds,size_t count) {
  if(!raw||!fds||count!=2)return false;auto& owner=*static_cast<Boundary*>(raw);std::lock_guard guard(owner.mutex);
  struct stat store{},lock{};std::string name,aux;
  if(!owner.Exclusive(sequence)||owner.window->operation!=1||owner.window->sealed||!owner.CurrentStore(owner.window->directory)||
     !owner.Outputs(fds[0],fds[1],&store,&lock,&name,&aux)||name!=owner.store_name||aux!=owner.lock_name||
     !owner.store_id.Same(store)||!owner.lock_id.Same(lock))return false;
  owner.window->sealed=true;owner.retire_only=true;return true;
}
uint32_t colossus_custody_descriptor_kind(void* raw,int fd) {
  if(!raw)return 0;auto& owner=*static_cast<Boundary*>(raw);std::lock_guard guard(owner.mutex);std::string path;
  if(!descriptor_path(fd,&path))return 0;
  if(path==owner.path+"/colossus-tls-identity")return 1;
  const std::string prefix=owner.path+"/colossus-tls-identity/";
  if(path==prefix+"colossus-tls-identity.keychain")return 2;
  if(path==prefix+"colossus-tls-identity.keychain-db")return 3;
  if(!owner.lock_name.empty()&&path==prefix+owner.lock_name)return 4;
  return 0;
}
bool colossus_custody_retired_suffix(void* raw,uint8_t* output) {
  if(!raw||!output)return false;auto& owner=*static_cast<Boundary*>(raw);std::lock_guard guard(owner.mutex);
  constexpr char prefix[]=".retired-tls-identity-";
  if(owner.retired_name.size()!=sizeof(prefix)-1+32)return false;
  memcpy(output,owner.retired_name.data()+sizeof(prefix)-1,32);return true;
}
void colossus_custody_end(void* raw,uint64_t sequence) {
  if(!raw)return;auto& owner=*static_cast<Boundary*>(raw);std::lock_guard guard(owner.mutex);
  if(owner.window&&owner.window->sequence==sequence)owner.window.reset();
}
void colossus_custody_drop(void* raw){delete static_cast<Boundary*>(raw);}
