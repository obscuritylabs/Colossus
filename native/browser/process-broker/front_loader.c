/* Isolated source for the future no-exec front. This is not built by the
 * current CEF staging target. Link only libSystem/libsandbox and the existing
 * pinned policy builder. The fixed verified body is a dylib with a private
 * colossus_owned_body_main entry; no CEF/Cocoa initializer precedes Seatbelt. */
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>
#include "image_verify.h"
#include "custody_probe.h"
#include <sys/socket.h>
#include <sys/time.h>
#if defined(NDEBUG)
#error "Owned custody fronts require an explicitly staged Debug development build"
#endif

#ifndef COLOSSUS_OWNED_MAIN_FRONT
#define COLOSSUS_OWNED_MAIN_FRONT 0
#endif

extern int sandbox_init_with_parameters(const char*,uint64_t,const char*const*,char**);
extern void sandbox_free_error(char*);

/* Native build supplies these pure fixed-image functions; they must not load
 * CEF, inspect argv for a program, or relax an inherited policy. A role policy
 * is regenerated from the pinned Chromium stock source with ALL process-exec
 * grants removed, plus the existing immutable network/filesystem obligations.
 * The source is sealed in the allocation manifest independently of the browser.
 * There is intentionally no implementation that falls back to stock CEF. */
extern int colossus_verified_owned_image(uint8_t role,char body[PATH_MAX]);
extern int colossus_final_owned_role_policy(uint8_t role,uint32_t browser_pid,char** source);

static int exact_read(int fd,void* data,size_t size) {
  unsigned char* cursor=data;
  while(size) { ssize_t count=read(fd,cursor,size);if(count<0&&errno==EINTR)continue;if(count<=0)return 0;cursor+=count;size-=(size_t)count; }
  return 1;
}
static uint32_t big_word(const uint8_t* bytes) { return ((uint32_t)bytes[0]<<24)|((uint32_t)bytes[1]<<16)|((uint32_t)bytes[2]<<8)|bytes[3]; }

int main(int argc,char** argv) {
  if(getuid()==0||getuid()!=geteuid()||argc<2||argc>129)return 70;
  /* Existing Darwin spawn already establishes a new POSIX session. Being its
   * leader makes setsid/setpgid escape fail under XNU's ordinary job-control
   * rules; audit-session identity is not used as domain membership. */
  if(getsid(0)!=getpid()||getpgrp()!=getpid())return 71;
  struct stat channel;uint8_t bootstrap[96];
  const int bootstrap_fd=COLOSSUS_OWNED_MAIN_FRONT?7:3;
  if(COLOSSUS_OWNED_MAIN_FRONT) {
    if(argc!=5)return 70;
    for(int i=1;i<5;++i)if(argv[i][0]!='0'+i+2||argv[i][1])return 70;
  }
  if(fstat(bootstrap_fd,&channel)!=0||
     !S_ISSOCK(channel.st_mode))return 72;
  struct timeval timeout={2,0};
  if(setsockopt(bootstrap_fd,SOL_SOCKET,SO_RCVTIMEO,&timeout,sizeof(timeout))||setsockopt(bootstrap_fd,SOL_SOCKET,SO_SNDTIMEO,&timeout,sizeof(timeout))||
     !exact_read(bootstrap_fd,bootstrap,sizeof(bootstrap))||memcmp(bootstrap,"CBOOT003",8))return 72;
  const uint32_t browser_pid=big_word(bootstrap+24); const uint8_t role=bootstrap[28];
  int nonce_nonzero=0;for(size_t i=8;i<24;++i)nonce_nonzero|=bootstrap[i]!=0;
  if(browser_pid<=1||!nonce_nonzero||
     (COLOSSUS_OWNED_MAIN_FRONT?(role!=0||browser_pid!=(uint32_t)getpid()):(role<1||role>4)))return 72;
  uint8_t mode=bootstrap[93];uint16_t extension_size=((uint16_t)bootstrap[94]<<8)|bootstrap[95];char extension[4097]={0};
  if(mode>1||extension_size>4096||(mode==0&&extension_size)||(mode==1&&!extension_size)||
     (extension_size&&(!exact_read(bootstrap_fd,extension,extension_size)||memchr(extension,0,extension_size))))return 72;
  struct colossus_custody_probe_binding binding;
  if(!colossus_owned_custody_probe_binding(&binding)||memcmp(binding.policy_sha256,bootstrap+61,32))return 73;
  char body[PATH_MAX];char* source=NULL;char* error=NULL;
  if(!colossus_verified_owned_image(role,body)||body[0]!='/'||!colossus_owned_library_verify(body,bootstrap+29)||
     !colossus_final_owned_role_policy(role,browser_pid,&source)||!source)return 73;
  const char* parameters[]={NULL};
  const int applied=sandbox_init_with_parameters(source,0,parameters,&error);
  free(source);if(error)sandbox_free_error(error);if(applied!=0)return 74;
  int probe=colossus_owned_custody_probe(bootstrap_fd,&binding,bootstrap+8,role,bootstrap+29,mode,extension);
  memset(extension,0,sizeof(extension));if(!probe)return 77;if(probe==2)return 0;
  if(!COLOSSUS_OWNED_MAIN_FRONT&&close(bootstrap_fd)!=0)return 72;
  /* No front→body exec occurs: the enrolled pidversion remains unchanged. The
   * final deny-fork/exec applies before dyld sees any body/CEF initializer. */
  void* library=dlopen(body,RTLD_NOW|RTLD_LOCAL);if(!library)return 75;
  typedef int (*body_main)(int,char**,uint32_t,const uint8_t*,uint8_t);
  body_main entry=(body_main)dlsym(library,"colossus_owned_body_main");
  if(!entry)return 76;
  const int result=entry(argc,argv,browser_pid,bootstrap+8,role);
  memset(bootstrap,0,sizeof(bootstrap));return result;
}
