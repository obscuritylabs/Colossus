// File-only tests of the actual owner state machine. Never create a Keychain,
// bind an item/code reference, or invoke any Security API. Main-image interpose
// tuples are inert; these tests exercise only retained POSIX file ownership.
#include <unistd.h>
#include <sys/wait.h>
#include <cassert>
#include <cerrno>
#include <cstdio>
#include <membership.h>
#include <uuid/uuid.h>

static bool fail_flush=false;
static int owned_fixture_fsync(int fd) {
  if (fail_flush) {fail_flush=false;errno=EIO;return -1;}
  return fsync(fd);
}
#define fsync owned_fixture_fsync
#include "profile_crypto.mm"
#undef fsync

static void setup(bool record) {
  char path[]="/private/tmp/colossus-profile-crypto-ownership.XXXXXX";
  assert(mkdtemp(path));
  State& owner=state(); owner.parent_path=path;
  owner.parent=open(path,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
  struct stat value{}; assert(fstat(owner.parent,&value)==0);owner.parent_id=identity(value);
  assert(mkdirat(owner.parent,kDirectory,0700)==0);owner.directory_created=true;
  owner.directory_path=owner.parent_path+"/"+kDirectory;
  owner.directory=openat(owner.parent,kDirectory,O_RDONLY|O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC);
  assert(fstat(owner.directory,&value)==0);owner.directory_id=identity(value);
  int file=openat(owner.directory,kStore,O_RDWR|O_CREAT|O_EXCL|O_NOFOLLOW|O_CLOEXEC,0600);
  assert(file>=0);assert(write(file,"fixture",7)==7);
  owner.retired_name=".retired-profile-crypto-fixture";
  if (record) {
    owner.file=file;owner.store_name=kStore;owner.store_path=owner.directory_path+"/"+kStore;
    assert(fstat(file,&value)==0);owner.file_id=identity(value);owner.store_recorded=true;
    owner.cleanup=Cleanup::Released;
  } else {close(file);}
  assert(namespace_valid(owner));
}
static void fixture_cleanup() {
  State& owner=state();
  // Remove only this test's own known fixture names after its assertions.
  for (const char* name : {kStore,"colossus-profile-crypto.keychain-db","keep-original"})
    unlinkat(owner.directory,name,0);
  for (const char* name : {".fl9F52894F","keep-aux-original"}) unlinkat(owner.directory,name,0);
  if (owner.aux>=0) {close(owner.aux);owner.aux=-1;}
  if (owner.file>=0) close(owner.file);
  close(owner.directory);owner.directory=-1;
  unlinkat(owner.parent,kDirectory,AT_REMOVEDIR);
  unlinkat(owner.parent,owner.retired_name.c_str(),AT_REMOVEDIR);
  close(owner.parent);owner.parent=-1;
  assert(rmdir(owner.parent_path.c_str())==0);
}
static void partial_creation() {
  setup(false);State& owner=state();owner.creation_unknown=true;
  assert(colossus_mac_profile_crypto_finish()==3);
  struct stat file{};assert(fstatat(owner.directory,kStore,&file,AT_SYMLINK_NOFOLLOW)==0);
  assert(owner.cleanup==Cleanup::Live);fixture_cleanup();
}
static void second_name() {
  setup(true);State& owner=state();
  int second=openat(owner.directory,"colossus-profile-crypto.keychain-db",O_CREAT|O_EXCL|O_RDWR|O_CLOEXEC,0600);
  assert(second>=0);close(second);
  assert(colossus_mac_profile_crypto_finish()==3);
  assert(owner.cleanup==Cleanup::Released);
  struct stat file{};assert(fstatat(owner.directory,kStore,&file,AT_SYMLINK_NOFOLLOW)==0);
  assert(fstatat(owner.directory,"colossus-profile-crypto.keychain-db",&file,AT_SYMLINK_NOFOLLOW)==0);
  fixture_cleanup();
}
static void replaced_record() {
  setup(true);State& owner=state();
  assert(renameat(owner.directory,kStore,owner.directory,"keep-original")==0);
  int replacement=openat(owner.directory,kStore,O_CREAT|O_EXCL|O_RDWR|O_CLOEXEC,0600);
  assert(replacement>=0);close(replacement);
  assert(colossus_mac_profile_crypto_finish()==3);
  struct stat file{};assert(fstatat(owner.directory,kStore,&file,AT_SYMLINK_NOFOLLOW)==0);
  assert(!owner.file_id.same(file));assert(owner.cleanup==Cleanup::Released);
  fixture_cleanup();
}
static void retry_flush() {
  setup(true);State& owner=state();std::string parent=owner.parent_path;
  fail_flush=true;
  assert(colossus_mac_profile_crypto_finish()==3);
  assert(owner.cleanup==Cleanup::DirectoryRemoved);
  assert(colossus_mac_profile_crypto_finish()==0);
  assert(owner.cleanup==Cleanup::Finished);
  assert(colossus_mac_profile_crypto_finish()==0);
  assert(rmdir(parent.c_str())==0);
}
static void prepare_after_finish() {
  assert(colossus_mac_profile_crypto_finish()==0);
  char path[]="/private/tmp/colossus-profile-crypto-finished.XXXXXX";
  assert(mkdtemp(path));
  const void* imports[4];
  for (size_t n=0;n<4;++n) imports[n]=interposes[n].replacement;
  assert(colossus_mac_profile_crypto_prepare(path,imports)==1);
  assert(!state().attempted);
  struct stat entry{};
  std::string candidate=std::string(path)+"/"+kDirectory;
  assert(lstat(candidate.c_str(),&entry)!=0 && errno==ENOENT);
  assert(rmdir(path)==0);
}
static void introduce_acl(int fd) {
  acl_t acl=acl_init(1);assert(acl);
  acl_entry_t entry{};assert(acl_create_entry(&acl,&entry)==0);
  assert(acl_set_tag_type(entry,ACL_EXTENDED_ALLOW)==0);
  uuid_t qualifier{};assert(mbr_uid_to_uuid(geteuid(),qualifier)==0);
  assert(acl_set_qualifier(entry,qualifier)==0);
  acl_permset_t permissions{};assert(acl_get_permset(entry,&permissions)==0);
  assert(acl_add_perm(permissions,ACL_READ_DATA)==0);
  assert(acl_set_fd_np(fd,acl,ACL_TYPE_EXTENDED)==0);acl_free(acl);
}
static void file_acl_change() {
  setup(true);State& owner=state();introduce_acl(owner.file);
  assert(!file_bound(owner));assert(colossus_mac_profile_crypto_finish()==3);
  assert(owner.cleanup==Cleanup::Released);fixture_cleanup();
}
static void parent_acl_change() {
  setup(true);State& owner=state();introduce_acl(owner.parent);
  assert(!parent_valid(owner));assert(colossus_mac_profile_crypto_finish()==3);
  struct stat record{};assert(fstatat(owner.directory,kStore,&record,AT_SYMLINK_NOFOLLOW)==0);
  assert(owner.cleanup==Cleanup::Released);fixture_cleanup();
}
static void setup_aux() {
  setup(true);State& owner=state();owner.aux_name=".fl9F52894F";
  owner.aux=openat(owner.directory,owner.aux_name.c_str(),O_CREAT|O_EXCL|O_RDWR|O_NOFOLLOW|O_CLOEXEC,0600);
  assert(owner.aux>=0);struct stat value{};assert(fstat(owner.aux,&value)==0);
  owner.aux_id=identity(value);owner.aux_recorded=true;
  assert(aux_bound(owner) && exact_entries(owner,true));
}
static void exact_aux_cleanup() {
  setup_aux();State& owner=state();const std::string parent=owner.parent_path;
  assert(colossus_mac_profile_crypto_finish()==0);
  assert(colossus_mac_profile_crypto_finish()==0);
  assert(owner.cleanup==Cleanup::Finished);assert(rmdir(parent.c_str())==0);
}
static void replaced_aux_preserved() {
  setup_aux();State& owner=state();
  assert(renameat(owner.directory,owner.aux_name.c_str(),owner.directory,"keep-aux-original")==0);
  int replacement=openat(owner.directory,owner.aux_name.c_str(),O_CREAT|O_EXCL|O_WRONLY|O_CLOEXEC,0600);
  assert(replacement>=0);close(replacement);
  assert(colossus_mac_profile_crypto_finish()==3);
  assert(owner.cleanup==Cleanup::Released);assert(!aux_bound(owner));fixture_cleanup();
}
static void nonempty_aux_preserved() {
  setup_aux();State& owner=state();assert(write(owner.aux,"x",1)==1);
  assert(!aux_bound(owner));assert(colossus_mac_profile_crypto_finish()==3);
  assert(owner.cleanup==Cleanup::Released);fixture_cleanup();
}
static void aux_acl_change_preserves_store() {
  setup_aux();State& owner=state();introduce_acl(owner.aux);
  assert(!aux_bound(owner));assert(colossus_mac_profile_crypto_finish()==3);
  assert(owner.cleanup==Cleanup::Released);
  struct stat original{};assert(fstatat(owner.directory,kStore,&original,AT_SYMLINK_NOFOLLOW)==0);
  assert(owner.file_id.same(original));fixture_cleanup();
}
static void store_fixture_not_profile_capability() {
  State& owner=state();owner.fixture_store_ready=true;
  assert(colossus_mac_profile_crypto_valid()==1);
  assert(!owner.ready && !owner.item);
}
int main() {
  for (auto test : {partial_creation,second_name,replaced_record,retry_flush,prepare_after_finish,file_acl_change,parent_acl_change,
                   exact_aux_cleanup,replaced_aux_preserved,nonempty_aux_preserved,aux_acl_change_preserves_store,store_fixture_not_profile_capability}) {
    pid_t child=fork();assert(child>=0);
    if (child==0) {test();_exit(0);}
    int status=0;assert(waitpid(child,&status,0)==child);assert(WIFEXITED(status)&&WEXITSTATUS(status)==0);
  }
  puts("PASS twelve file-only profile crypto ownership/auxiliary regressions; no Security API calls");
}
