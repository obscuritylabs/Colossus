/* Executed by the libSystem/libsandbox-only front after policy and BEFORE any
 * dylib/body/CEF initializer. Only fixed disposable plain markers are touched. */
#include "custody_probe.h"
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <unistd.h>
#include <servers/bootstrap.h>
#include <mach/mach.h>
extern char* sandbox_extension_issue_file(const char*,const char*,unsigned int);
extern int64_t sandbox_extension_consume(const char*);
extern int sandbox_extension_release(int64_t);
static int exact(int fd,void* data,size_t size,int write_mode) {
  unsigned char* cursor=data;while(size){ssize_t count=write_mode?write(fd,cursor,size):read(fd,cursor,size);
    if(count<0&&errno==EINTR)continue;if(count<=0)return 0;cursor+=count;size-=(size_t)count;}return 1;
}
static int path(char output[PATH_MAX],const char* root,const char* name) {
  int count=snprintf(output,PATH_MAX,"%s/%s",root,name);return count>0&&count<PATH_MAX;
}
static int denied(int result){return result<0&&(errno==EACCES||errno==EPERM);}
static int opened_denied(const char* name,int flags) {
  errno=0;int fd=open(name,flags|O_NOFOLLOW|O_CLOEXEC,0600);int result=denied(fd);
  if(fd>=0)close(fd);return result;
}
static int service_denied(const char* name) {
  mach_port_t result=MACH_PORT_NULL;
  kern_return_t status=bootstrap_look_up(bootstrap_port,name,&result);
  if(status==KERN_SUCCESS&&result!=MACH_PORT_NULL)mach_port_deallocate(mach_task_self(),result);
  /* Absence, unknown service and generic errors do not prove an OS denial. */
  return status==BOOTSTRAP_NOT_PRIVILEGED;
}
int colossus_owned_custody_probe(int fd,const struct colossus_custody_probe_binding* binding,
    const uint8_t nonce[16],uint8_t role,const uint8_t body[32],uint8_t mode,const char* extension) {
  if(fd<0||!binding||!nonce||!body||role>4||mode>1||!extension||(mode==0&&*extension)||(mode==1&&!*extension))return 0;
  char read_marker[PATH_MAX],write_marker[PATH_MAX],rename_marker[PATH_MAX],unlink_marker[PATH_MAX],link_marker[PATH_MAX],absent[PATH_MAX],alias[PATH_MAX];
  if(!path(read_marker,binding->root,".colossus-custody-probe-read")||!path(write_marker,binding->root,".colossus-custody-probe-write")||
     !path(rename_marker,binding->root,".colossus-custody-probe-rename")||!path(unlink_marker,binding->root,".colossus-custody-probe-unlink")||
     !path(link_marker,binding->root,".colossus-custody-probe-link")||!path(absent,binding->root,".colossus-custody-probe-create")||
     !path(alias,binding->profile,".colossus-custody-probe-alias"))return 0;
  if(!opened_denied(read_marker,O_RDONLY)||!opened_denied(write_marker,O_WRONLY)||!opened_denied(absent,O_WRONLY|O_CREAT|O_EXCL))return 0;
  errno=0;if(!denied(rename(rename_marker,absent)))return 0;
  errno=0;if(!denied(unlink(unlink_marker)))return 0;
  errno=0;if(!denied(link(link_marker,alias)))return 0;
  errno=0;char* issued=sandbox_extension_issue_file("com.apple.app-sandbox.read-write",read_marker,0);
  if(issued){free(issued);return 0;}if(errno!=EACCES&&errno!=EPERM)return 0;
  if(!service_denied("com.apple.SecurityServer")||!service_denied("com.apple.securityd.xpc")||!service_denied("com.apple.trustd.agent"))return 0;
  uint16_t flags=0x00ff;
  if(mode==1) {
    int64_t handle=sandbox_extension_consume(extension);if(handle<0)return 0;
    int rejected=opened_denied(read_marker,O_RDONLY);int released=sandbox_extension_release(handle)==0;
    if(!rejected||!released)return 0;flags|=0x0100;
  }
  uint8_t receipt[92]={0};memcpy(receipt,"CCUS0001",8);memcpy(receipt+8,nonce,16);receipt[24]=role;
  memcpy(receipt+25,binding->policy_sha256,32);memcpy(receipt+57,body,32);receipt[89]=(uint8_t)(flags>>8);receipt[90]=(uint8_t)flags;receipt[91]=mode;
  if(!exact(fd,receipt,sizeof(receipt),1))return 0;
  uint8_t ack[26];if(!exact(fd,ack,sizeof(ack),0)||memcmp(ack,"CCAK0001",8)||memcmp(ack+8,nonce,16)||ack[24]!=role||ack[25]!=mode)return 0;
  memset(receipt,0,sizeof(receipt));return mode==1?2:1;
}
