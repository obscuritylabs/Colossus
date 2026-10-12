/* Pure pinned builder integration for the isolated no-exec frontend. This file
 * does not load CEF, Cocoa, Security.framework, or arbitrary browser policy. */
#include "policy_transition.h"
#include "custody_policy.h"
#include "custody_probe.h"
#include <CommonCrypto/CommonDigest.h>
#include "../mac/network_envelope.h"
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <mach-o/dyld.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/sysctl.h>
#include <unistd.h>

int colossus_verified_owned_image(uint8_t role,char body[PATH_MAX]) {
  if(role>4)return 0;
  char raw[PATH_MAX],front[PATH_MAX];uint32_t size=sizeof(raw);
  if(_NSGetExecutablePath(raw,&size)||!realpath(raw,front))return 0;
  int length=snprintf(body,PATH_MAX,"%s Body.dylib",front);return length>0&&length<PATH_MAX;
}
static int resource(char bundle[PATH_MAX],char storage[32769],struct colossus_mac_network_binding* binding,uint8_t* digest) {
  char raw[PATH_MAX],front[PATH_MAX];uint32_t size=sizeof(raw);
  if(_NSGetExecutablePath(raw,&size)||!realpath(raw,front))return 0;
  const char* nested=strstr(front,"/Contents/Frameworks/");
  const char* main=strstr(front,"/Contents/MacOS/");const char* split=nested?nested:main;
  if(!split||split==front)return 0;size_t prefix=(size_t)(split-front);
  memcpy(bundle,front,prefix);bundle[prefix]=0;
  char path[PATH_MAX];int length=snprintf(path,sizeof(path),"%s/Contents/Resources/colossus-network-envelope.policy",bundle);
  if(length<=0||length>=(int)sizeof(path))return 0;
  int fd=open(path,O_RDONLY|O_NOFOLLOW|O_CLOEXEC);if(fd<0)return 0;
  struct stat before,after;int valid=fstat(fd,&before)==0&&S_ISREG(before.st_mode)&&before.st_uid==geteuid()&&before.st_nlink==1&&(before.st_mode&07777)==0400&&before.st_size>0&&before.st_size<=32768;
  size_t remaining=valid?(size_t)before.st_size:0;char* cursor=storage;
  while(valid&&remaining){ssize_t count=read(fd,cursor,remaining);if(count<0&&errno==EINTR)continue;if(count<=0){valid=0;break;}cursor+=count;remaining-=(size_t)count;}
  valid=valid&&fstat(fd,&after)==0&&before.st_dev==after.st_dev&&before.st_ino==after.st_ino&&before.st_size==after.st_size&&before.st_mtimespec.tv_sec==after.st_mtimespec.tv_sec&&before.st_mtimespec.tv_nsec==after.st_mtimespec.tv_nsec;
  if(close(fd))valid=0;if(!valid)return 0;storage[before.st_size]=0;
  if(digest&&!CC_SHA256(storage,(CC_LONG)before.st_size,digest))return 0;
  return colossus_mac_network_binding_parse(storage,(size_t)before.st_size,binding)&&!strcmp(bundle,binding->bundle_path);
}
int colossus_owned_custody_probe_binding(struct colossus_custody_probe_binding* output) {
  if(!output)return 0;char bundle[PATH_MAX],storage[32769];struct colossus_mac_network_binding binding;
  if(!resource(bundle,storage,&binding,output->policy_sha256)||strlen(binding.broker_root)>=PATH_MAX||strlen(binding.profile_root)>=PATH_MAX)return 0;
  strcpy(output->root,binding.broker_root);strcpy(output->profile,binding.profile_root);memset(storage,0,sizeof(storage));return 1;
}
static uint32_t os_version(void) {
  char version[32];size_t size=sizeof(version);if(sysctlbyname("kern.osproductversion",version,&size,NULL,0)||size==0||size>sizeof(version))return 0;
  version[sizeof(version)-1]=0;unsigned major=0,minor=0;if(sscanf(version,"%u.%u",&major,&minor)<1||major>99||minor>99)return 0;return major*100+minor;
}
int colossus_final_owned_role_policy(uint8_t role,uint32_t browser_pid,char** source) {
  *source=NULL;char bundle[PATH_MAX],storage[32769];struct colossus_mac_network_binding binding;
  if(role>4||browser_pid<=1||browser_pid>INT32_MAX||!resource(bundle,storage,&binding,NULL))return 0;
  char* pinned=NULL;int built=0;
  if(role==0) {
    static const char* suffixes[]={""," (Alerts)"," (GPU)"," (Plugin)"," (Renderer)"};char paths[5][PATH_MAX];const char* fronts[5];
    for(size_t i=0;i<5;++i){int count=snprintf(paths[i],sizeof(paths[i]),"%s/Contents/Frameworks/Colossus Browser Helper%s.app/Contents/MacOS/Colossus Browser Helper%s",bundle,suffixes[i],suffixes[i]);if(count<=0||count>=PATH_MAX)return 0;fronts[i]=paths[i];}
    built=colossus_mac_main_network_policy(&binding,fronts,5,&pinned);
  } else {
    static const enum colossus_mac_network_role roles[]={COLOSSUS_MAC_ROLE_RENDERER,COLOSSUS_MAC_ROLE_RENDERER,COLOSSUS_MAC_ROLE_GPU,COLOSSUS_MAC_ROLE_NETWORK,COLOSSUS_MAC_ROLE_UTILITY};
    char raw[PATH_MAX],front[PATH_MAX];uint32_t size=sizeof(raw);if(_NSGetExecutablePath(raw,&size)||!realpath(raw,front))return 0;
    uint32_t version=os_version();if(!version)return 0;
    built=colossus_mac_helper_policy(roles[role],&binding,front,(int32_t)browser_pid,version,&pinned);
  }
  if(!built||!pinned)return 0;
  char* no_process=NULL;
  int transformed=colossus_owned_process_policy(pinned,strlen(pinned),&no_process);free(pinned);
  if(transformed)transformed=colossus_owned_custody_policy(no_process,strlen(no_process),binding.broker_root,source);
  free(no_process);
  if(transformed&&role==0) {
    /* Only Main registers its own one-shot Mojo server. The browser cannot
     * create arbitrary jobs/services through its inherited bootstrap port. */
    char grant[1024];int length=snprintf(grant,sizeof(grant),"(allow mach-register (global-name \"%s.MachPortRendezvousServer.%u\"))\n",binding.bundle_id,browser_pid);
    size_t used=strlen(*source);
    if(length<=0||length>=(int)sizeof(grant)||used+(size_t)length>128*1024) {free(*source);*source=NULL;transformed=0;}
    else {memcpy(*source+used,grant,(size_t)length+1);}
  }
  memset(storage,0,sizeof(storage));return transformed;
}
