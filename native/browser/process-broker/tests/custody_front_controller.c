/* Explicit signed operator fixture controller. Never linked into CEF/Core.
 * Launches one actual start-suspended front, validates signed direct child,
 * permits only fixed probe-only exit, and reaps it. No Keychain/audit API. */
#include <Security/Security.h>
#include <CommonCrypto/CommonDigest.h>
#include <servers/bootstrap.h>
#include <mach/mach.h>
#include <spawn.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/random.h>
#include <sys/wait.h>
#include <signal.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <limits.h>
extern char* sandbox_extension_issue_file(const char*,const char*,unsigned int);
static int hex(const char* text,uint8_t* result,size_t size) {
  if(strlen(text)!=size*2)return 0;for(size_t i=0;i<size;++i){unsigned value=0;if(sscanf(text+i*2,"%2x",&value)!=1)return 0;result[i]=(uint8_t)value;}return 1;
}
static int exact(int fd,void* bytes,size_t size,int writing){uint8_t* p=bytes;while(size){ssize_t n=writing?write(fd,p,size):read(fd,p,size);if(n<0&&errno==EINTR)continue;if(n<=0)return 0;p+=n;size-=(size_t)n;}return 1;}
static int positive_service(const char* name){mach_port_t service=MACH_PORT_NULL;kern_return_t result=bootstrap_look_up(bootstrap_port,name,&service);if(result==KERN_SUCCESS&&service!=MACH_PORT_NULL){mach_port_deallocate(mach_task_self(),service);return 1;}return 0;}
static int signed_child(pid_t child,const char* cdhash) {
  int pid=child;CFNumberRef number=CFNumberCreate(kCFAllocatorDefault,kCFNumberIntType,&pid);
  const void* keys[]={kSecGuestAttributePid};const void* values[]={number};
  CFDictionaryRef attributes=CFDictionaryCreate(kCFAllocatorDefault,keys,values,1,&kCFTypeDictionaryKeyCallBacks,&kCFTypeDictionaryValueCallBacks);
  char expression[64];snprintf(expression,sizeof(expression),"cdhash H\"%s\"",cdhash);
  CFStringRef text=CFStringCreateWithCString(kCFAllocatorDefault,expression,kCFStringEncodingASCII);SecCodeRef code=NULL;SecRequirementRef requirement=NULL;
  int valid=number&&attributes&&text&&SecCodeCopyGuestWithAttributes(NULL,attributes,kSecCSDefaultFlags,&code)==errSecSuccess&&
      SecRequirementCreateWithString(text,kSecCSDefaultFlags,&requirement)==errSecSuccess&&SecCodeCheckValidity(code,kSecCSStrictValidate|kSecCSNoNetworkAccess,requirement)==errSecSuccess;
  if(requirement)CFRelease(requirement);if(code)CFRelease(code);if(text)CFRelease(text);if(attributes)CFRelease(attributes);if(number)CFRelease(number);return valid;
}
int main(int argc,char** argv) {
  if(argc!=7||getuid()==0||geteuid()!=getuid())return 64;
  char canonical[PATH_MAX];if(!realpath(argv[1],canonical)||strcmp(canonical,argv[1]))return 64;
  char* end=NULL;long role=strtol(argv[2],&end,10);uint8_t body[32],policy[32],front[20];
  if(!end||*end||role<0||role>4||!hex(argv[3],front,20)||!hex(argv[4],body,32)||!hex(argv[5],policy,32))return 64;
  int marker=open(argv[6],O_RDONLY|O_NOFOLLOW|O_CLOEXEC);struct stat before,after;
  if(marker<0||fstat(marker,&before)||!S_ISREG(before.st_mode)||before.st_uid!=geteuid()||before.st_nlink!=1||(before.st_mode&07777)!=0600)return 65;
  char* extension=sandbox_extension_issue_file("com.apple.app-sandbox.read",argv[6],0);
  if(!extension||strlen(extension)>4096){fputs("probe category=valid_extension_unavailable\n",stderr);return 66;}
  if(!positive_service("com.apple.SecurityServer")||!positive_service("com.apple.securityd.xpc")||!positive_service("com.apple.trustd.agent")){fputs("probe category=security_service_baseline_unavailable\n",stderr);free(extension);return 67;}
  int pair[2];if(socketpair(AF_UNIX,SOCK_STREAM,0,pair))return 68;
  int source=fcntl(pair[1],F_DUPFD_CLOEXEC,32);close(pair[1]);if(source<0)return 68;
  struct timeval timeout={4,0};setsockopt(pair[0],SOL_SOCKET,SO_RCVTIMEO,&timeout,sizeof(timeout));setsockopt(pair[0],SOL_SOCKET,SO_SNDTIMEO,&timeout,sizeof(timeout));
  posix_spawn_file_actions_t actions;posix_spawnattr_t attributes;
  if(posix_spawn_file_actions_init(&actions)||posix_spawnattr_init(&attributes))return 68;
  posix_spawn_file_actions_addopen(&actions,0,"/dev/null",O_RDONLY,0);posix_spawn_file_actions_addopen(&actions,1,"/dev/null",O_WRONLY,0);posix_spawn_file_actions_addopen(&actions,2,"/dev/null",O_WRONLY,0);
  int target=role==0?7:3;posix_spawn_file_actions_adddup2(&actions,source,target);
  if(role==0)for(int fd=3;fd<7;++fd)posix_spawn_file_actions_adddup2(&actions,source,fd);
  const short flags=POSIX_SPAWN_START_SUSPENDED|POSIX_SPAWN_SETSID|POSIX_SPAWN_CLOEXEC_DEFAULT;
  posix_spawnattr_setflags(&attributes,flags);
  char* main_args[]={argv[1],"3","4","5","6",NULL};char* helper_args[]={argv[1],"--type=utility",NULL};
  char* environment[]={"PATH=/usr/bin:/bin","LANG=en_US.UTF-8","MallocNanoZone=0",NULL};pid_t child=0;
  int spawned=posix_spawn(&child,argv[1],&actions,&attributes,role==0?main_args:helper_args,environment);
  posix_spawn_file_actions_destroy(&actions);posix_spawnattr_destroy(&attributes);close(source);
  if(spawned||child<=1){free(extension);return 69;}
  // Retained actual direct child, still kernel-stopped: PID cannot be adopted,
  // recycled or exec before exact signed image validation and bootstrap.
  int accepted=0,status=0,reaped=0;uint8_t nonce[16],header[96]={0},receipt[92],ack[26]={0};
  if(!signed_child(child,argv[3])||getentropy(nonce,sizeof(nonce)))goto cleanup;
  memcpy(header,"CBOOT003",8);memcpy(header+8,nonce,16);uint32_t browser=role==0?(uint32_t)child:(uint32_t)getpid();
  header[24]=(uint8_t)(browser>>24);header[25]=(uint8_t)(browser>>16);header[26]=(uint8_t)(browser>>8);header[27]=(uint8_t)browser;header[28]=(uint8_t)role;
  memcpy(header+29,body,32);memcpy(header+61,policy,32);header[93]=1;size_t size=strlen(extension);header[94]=(uint8_t)(size>>8);header[95]=(uint8_t)size;
  if(!exact(pair[0],header,sizeof(header),1)||!exact(pair[0],extension,size,1)||kill(child,SIGCONT))goto cleanup;
  if(!exact(pair[0],receipt,sizeof(receipt),0)||memcmp(receipt,"CCUS0001",8)||memcmp(receipt+8,nonce,16)||receipt[24]!=(uint8_t)role||
     memcmp(receipt+25,policy,32)||memcmp(receipt+57,body,32)||receipt[89]!=1||receipt[90]!=255||receipt[91]!=1||!signed_child(child,argv[3]))goto cleanup;
  memcpy(ack,"CCAK0001",8);memcpy(ack+8,nonce,16);ack[24]=(uint8_t)role;ack[25]=1;
  if(!exact(pair[0],ack,sizeof(ack),1))goto cleanup;
  for(unsigned i=0;i<400;++i){pid_t result=waitpid(child,&status,WNOHANG);if(result==child){reaped=1;break;}if(result<0&&errno!=EINTR)break;struct timespec delay={0,10000000};nanosleep(&delay,NULL);}
  if(reaped&&WIFEXITED(status)&&WEXITSTATUS(status)==0&&fstat(marker,&after)==0&&before.st_dev==after.st_dev&&before.st_ino==after.st_ino&&after.st_nlink==1&&before.st_size==after.st_size)accepted=1;
cleanup:
  if(!reaped){kill(child,SIGKILL);while(waitpid(child,&status,0)<0&&errno==EINTR){} }
  free(extension);close(marker);close(pair[0]);
  if(!accepted){fprintf(stderr,"probe category=denial_or_native_front_exit_unknown role=%ld exit=%d\n",role,WIFEXITED(status)?WEXITSTATUS(status):-1);return 70;}
  printf("{\"schema_version\":1,\"role\":%ld,\"pre_body_denials\":511,\"direct_child_reaped\":true,\"exit_code\":0,\"cef_loaded\":false,\"production_accepted\":false}\n",role);return 0;
}
