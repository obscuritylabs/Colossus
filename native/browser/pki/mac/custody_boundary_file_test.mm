// Plain-file descriptor-generation tests only. No Security, Keychain, browser,
// process spawning, OS sandbox application, audit token or certificate APIs.
#include "custody_boundary.h"
#include <CommonCrypto/CommonDigest.h>
#include <cassert>
#include <array>
#include <fcntl.h>
#include <sys/stat.h>
#include <unistd.h>
#include <cstdio>
#include <cstring>
#include <string>
static int file(int directory,const char* name,const char* data) {
  int fd=openat(directory,name,O_CREAT|O_EXCL|O_RDWR|O_CLOEXEC,0600);assert(fd>=0);
  assert(write(fd,data,strlen(data))==static_cast<ssize_t>(strlen(data)));return fd;
}
int main() {
  char raw[]="/private/tmp/colossus-custody-file-XXXXXX";assert(mkdtemp(raw));
  std::string root=raw;assert(chmod(raw,0700)==0);int parent=open(raw,O_DIRECTORY|O_RDONLY|O_CLOEXEC);assert(parent>=0);
  std::array<uint8_t,32> generation{};assert(!colossus_custody_create(raw,generation.data()));generation.fill(9);
  void* owner=colossus_custody_create(raw,generation.data());assert(owner);
  assert(colossus_custody_begin(owner,generation.data(),parent,-1,1,3));
  assert(!colossus_custody_begin(owner,generation.data(),parent,-1,2,3));
  assert(mkdirat(parent,"colossus-tls-identity",0700)==0);
  int directory=openat(parent,"colossus-tls-identity",O_DIRECTORY|O_RDONLY|O_NOFOLLOW|O_CLOEXEC);assert(directory>=0);
  int namespace_fds[]={directory};assert(colossus_custody_seal(owner,1,namespace_fds,1));
  assert(!colossus_custody_seal(owner,1,namespace_fds,1));colossus_custody_end(owner,1);
  assert(!colossus_custody_begin(owner,generation.data(),parent,directory,1,0));
  auto foreign=generation;foreign[1]^=1;assert(!colossus_custody_begin(owner,foreign.data(),parent,directory,2,0));
  assert(colossus_custody_begin(owner,generation.data(),parent,directory,3,0));
  constexpr char store_name[]="colossus-tls-identity.keychain-db";std::array<uint8_t,20> digest{};assert(CC_SHA1(store_name,sizeof(store_name)-1,digest.data()));
  char lock_name[12];snprintf(lock_name,sizeof(lock_name),".fl%02X%02X%02X%02X",digest[0],digest[1],digest[2],digest[3]);
  int store=file(directory,store_name,"synthetic store");int lock=file(directory,lock_name,"");int outputs[]={store,lock};
  int extra=file(directory,"foreign","");assert(!colossus_custody_seal(owner,3,outputs,2));assert(close(extra)==0);assert(unlinkat(directory,"foreign",0)==0);
  assert(colossus_custody_seal(owner,3,outputs,2));colossus_custody_end(owner,3);
  // Only a positively admitted import may replace the store generation.
  assert(colossus_custody_begin(owner,generation.data(),parent,directory,4,1));
  int replacement=file(directory,"replacement","replacement");assert(renameat(directory,"replacement",directory,store_name)==0);
  int new_outputs[]={replacement,lock};assert(colossus_custody_seal(owner,4,new_outputs,2));assert(!colossus_custody_seal(owner,4,new_outputs,2));colossus_custody_end(owner,4);
  assert(colossus_custody_begin(owner,generation.data(),parent,directory,5,1));
  assert(linkat(directory,store_name,parent,"alias",0)==0);assert(!colossus_custody_seal(owner,5,new_outputs,2));assert(unlinkat(parent,"alias",0)==0);
  colossus_custody_end(owner,5);
  // A foreign replacement between operations cannot open a mutation window.
  int alien=file(directory,"alien","unadmitted");assert(renameat(directory,"alien",directory,store_name)==0);
  assert(!colossus_custody_begin(owner,generation.data(),parent,directory,6,1));
  assert(!colossus_custody_begin(owner,generation.data(),parent,directory,7,2));
  colossus_custody_drop(owner);
  // Test-owned synthetic files only; no native key/store cleanup claim.
  assert(close(store)==0&&close(replacement)==0&&close(alien)==0&&close(lock)==0&&close(directory)==0);
  assert(unlinkat(parent,("colossus-tls-identity/"+std::string(store_name)).c_str(),0)==0);
  assert(unlinkat(parent,("colossus-tls-identity/"+std::string(lock_name)).c_str(),0)==0);
  assert(unlinkat(parent,"colossus-tls-identity",AT_REMOVEDIR)==0&&close(parent)==0&&rmdir(raw)==0);
  // Unknown contents may be retired only in the SAME old sealed physical files.
  // The native proof rejects replaced descriptors and extra namespace entries,
  // then independently fences this owner against any future create/import.
  char second[]="/private/tmp/colossus-custody-unchanged-XXXXXX";assert(mkdtemp(second));
  assert(chmod(second,0700)==0);parent=open(second,O_DIRECTORY|O_RDONLY|O_CLOEXEC);assert(parent>=0);
  owner=colossus_custody_create(second,generation.data());assert(owner);
  assert(colossus_custody_begin(owner,generation.data(),parent,-1,1,3));
  assert(mkdirat(parent,"colossus-tls-identity",0700)==0);
  directory=openat(parent,"colossus-tls-identity",O_DIRECTORY|O_RDONLY|O_NOFOLLOW|O_CLOEXEC);assert(directory>=0);
  const int next_namespace[]={directory};assert(colossus_custody_seal(owner,1,next_namespace,1));colossus_custody_end(owner,1);
  assert(colossus_custody_begin(owner,generation.data(),parent,directory,2,0));
  store=file(directory,store_name,"old owned physical file");lock=file(directory,lock_name,"");const int old_fds[]={store,lock};
  assert(colossus_custody_seal(owner,2,old_fds,2));colossus_custody_end(owner,2);
  assert(colossus_custody_begin(owner,generation.data(),parent,directory,3,1));
  assert(pwrite(store,"unknown imported contents",25,0)==25);
  extra=file(directory,"unowned-extra","");assert(!colossus_custody_retire_unchanged(owner,3,old_fds,2));
  assert(close(extra)==0&&unlinkat(directory,"unowned-extra",0)==0);
  assert(colossus_custody_retire_unchanged(owner,3,old_fds,2));
  assert(!colossus_custody_retire_unchanged(owner,3,old_fds,2));colossus_custody_end(owner,3);
  assert(!colossus_custody_begin(owner,generation.data(),parent,directory,4,1));
  assert(colossus_custody_begin(owner,generation.data(),parent,directory,5,2));
  constexpr char retired[]=".retired-tls-identity-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
  assert(renameat(parent,"colossus-tls-identity",parent,retired)==0);
  assert(colossus_custody_checkpoint(owner,5,0));
  assert(!colossus_custody_checkpoint(owner,5,2));
  assert(unlinkat(directory,store_name,0)==0&&colossus_custody_checkpoint(owner,5,1));
  assert(unlinkat(directory,lock_name,0)==0&&colossus_custody_checkpoint(owner,5,2));
  assert(!colossus_custody_checkpoint(owner,5,3));
  assert(unlinkat(parent,retired,AT_REMOVEDIR)==0&&colossus_custody_checkpoint(owner,5,3));
  colossus_custody_end(owner,5);
  colossus_custody_drop(owner);
  assert(close(store)==0&&close(lock)==0&&close(directory)==0);
  assert(close(parent)==0&&rmdir(second)==0);
}
