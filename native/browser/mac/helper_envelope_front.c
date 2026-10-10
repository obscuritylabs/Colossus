/* The helper's only unsandboxed entry. Link libSystem and libsandbox only.
 * No C++/Objective-C, CEF, Cocoa or Security framework may initialize here. */
#include "network_envelope.h"
#include <bsm/audit.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <mach-o/dyld.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/sysctl.h>
#include <unistd.h>

#ifndef COLOSSUS_MAC_HELPER_KIND
#error "A fixed sealed helper kind is required (base=0, renderer=1, gpu=2, plugin=3, alerts=4)."
#endif

extern char** environ;
extern int sandbox_init_with_parameters(const char*, uint64_t, const char* const*, char**);
extern void sandbox_free_error(char*);
static int decimal(const char* s,int* result) {
  if(!*s || strlen(s)>5 || (*s=='0' && s[1]))return 0;
  unsigned n=0; for(const char* c=s;*c;++c){if(*c<'0'||*c>'9')return 0;n=n*10+(unsigned)(*c-'0');}
  if(n<3 || n>65535)return 0; *result=(int)n; return 1;
}
static int denied_switch(const char* a) {
  static const char* names[]={"no-sandbox","disable-gpu-sandbox","disable-seccomp-filter-sandbox",
    "disable-setuid-sandbox","allow-sandbox-debugging","single-process","in-process-gpu",
    "renderer-cmd-prefix","utility-cmd-prefix","zygote-cmd-prefix","gpu-launcher",
    "browser-subprocess-path","launch-as-browser","remote-debugging-port","remote-debugging-pipe"};
  for(size_t i=0;i<sizeof(names)/sizeof(names[0]);++i){size_t n=strlen(names[i]);
    if(strncmp(a+2,names[i],n)==0 && (a[n+2]==0 || a[n+2]=='='))return 1;}
  return 0;
}
static int arguments(int argc,char** argv,enum colossus_mac_network_role* role,int* policy_fd) {
  if(argc<2 || argc>256)return 0;
  const char* type=NULL; const char* service=NULL;
  *policy_fd=-1;
  for(int i=1;i<argc;++i) {
    const char* a=argv[i]; size_t n=strnlen(a,4097);
    if(n<3 || n>4096 || a[0]!='-' || a[1]!='-' || denied_switch(a))return 0;
    for(size_t j=0;j<n;++j)if((unsigned char)a[j]<32 || (unsigned char)a[j]>126)return 0;
    if(strncmp(a,"--type=",7)==0){if(type)return 0;type=a+7;}
    if(strncmp(a,"--service-sandbox-type=",23)==0){if(service)return 0;service=a+23;}
    if(strncmp(a,"--seatbelt-client=",18)==0){if(*policy_fd!=-1||!decimal(a+18,policy_fd))return 0;}
  }
  if(!type || *policy_fd<3)return 0;
#if COLOSSUS_MAC_HELPER_KIND == 1
  if(strcmp(type,"renderer") || service)return 0; *role=COLOSSUS_MAC_ROLE_RENDERER; return 1;
#elif COLOSSUS_MAC_HELPER_KIND == 2
  if(strcmp(type,"gpu-process") || service)return 0; *role=COLOSSUS_MAC_ROLE_GPU; return 1;
#else
  if(strcmp(type,"utility") || !service)return 0;
  struct item { const char* name; enum colossus_mac_network_role role; };
  static const struct item roles[]={
    {"network",COLOSSUS_MAC_ROLE_NETWORK},{"utility",COLOSSUS_MAC_ROLE_UTILITY},
    {"service",COLOSSUS_MAC_ROLE_UTILITY},{"service_with_jit",COLOSSUS_MAC_ROLE_UTILITY},
    {"audio",COLOSSUS_MAC_ROLE_AUDIO},{"cdm",COLOSSUS_MAC_ROLE_CDM},
    {"mirroring",COLOSSUS_MAC_ROLE_MIRRORING},{"print_backend",COLOSSUS_MAC_ROLE_PRINT_BACKEND},
    {"print_compositor",COLOSSUS_MAC_ROLE_PRINT_COMPOSITOR},{"proxy_resolver",COLOSSUS_MAC_ROLE_PROXY_RESOLVER},
    {"screen_ai",COLOSSUS_MAC_ROLE_SCREEN_AI},{"speech_recognition",COLOSSUS_MAC_ROLE_SPEECH_RECOGNITION},
    {"on_device_model_execution",COLOSSUS_MAC_ROLE_ON_DEVICE_MODEL},
    {"on_device_translation",COLOSSUS_MAC_ROLE_ON_DEVICE_TRANSLATION},
    {"webnn_model_compilation",COLOSSUS_MAC_ROLE_WEBNN}};
  for(size_t i=0;i<sizeof(roles)/sizeof(roles[0]);++i)if(strcmp(service,roles[i].name)==0){
#if COLOSSUS_MAC_HELPER_KIND == 3
    if(roles[i].role!=COLOSSUS_MAC_ROLE_CDM)return 0;
#elif COLOSSUS_MAC_HELPER_KIND == 4
    if(roles[i].role!=COLOSSUS_MAC_ROLE_UTILITY)return 0;
#endif
    *role=roles[i].role; return 1;
  }
  return 0;
#endif
}
static int exact_read(int fd,void* output,size_t size) {
  unsigned char* b=output;
  while(size){ssize_t n=read(fd,b,size);if(n<0&&errno==EINTR)continue;if(n<=0)return 0;b+=n;size-=(size_t)n;}
  return 1;
}
static int discard_browser_policy(int fd) {
  struct stat metadata;
  if(fstat(fd,&metadata)!=0 || !S_ISFIFO(metadata.st_mode))return 0;
  uint64_t size=0;
  if(!exact_read(fd,&size,sizeof(size)) || !size || size>128*1024)return 0;
  char bytes[4096];
  while(size){size_t n=size>sizeof(bytes)?sizeof(bytes):(size_t)size;if(!exact_read(fd,bytes,n))return 0;size-=n;}
  return close(fd)==0;
}
static int canonical(const char* input,char output[PATH_MAX]) {
  return realpath(input,output) && strcmp(input,output)==0;
}
static int resource(const char* bundle,char* storage,size_t capacity,size_t* size) {
  char name[PATH_MAX];
  int n=snprintf(name,sizeof(name),"%s/Contents/Resources/colossus-network-envelope.policy",bundle);
  if(n<0||(size_t)n>=sizeof(name))return 0;
  int fd=open(name,O_RDONLY|O_CLOEXEC|O_NOFOLLOW);
  if(fd<0)return 0;
  struct stat before,after;
  int ok=fstat(fd,&before)==0 && S_ISREG(before.st_mode) && before.st_uid==geteuid() &&
    before.st_nlink==1 && (before.st_mode&07777)==0400 && before.st_size>0 &&
    (uint64_t)before.st_size<capacity;
  if(ok)ok=exact_read(fd,storage,(size_t)before.st_size) && fstat(fd,&after)==0 &&
    before.st_dev==after.st_dev && before.st_ino==after.st_ino && before.st_size==after.st_size &&
    before.st_mtimespec.tv_sec==after.st_mtimespec.tv_sec && before.st_mtimespec.tv_nsec==after.st_mtimespec.tv_nsec;
  if(ok){*size=(size_t)before.st_size;storage[*size]=0;}
  if(close(fd)!=0)ok=0;
  return ok;
}
static uint32_t os_version(void) {
  char value[64]; size_t n=sizeof(value);
  if(sysctlbyname("kern.osproductversion",value,&n,NULL,0)!=0 || n==0 || n>sizeof(value))return 0;
  value[sizeof(value)-1]=0; unsigned major=0,minor=0;
  if(sscanf(value,"%u.%u",&major,&minor)<1 || major<14 || major>99 || minor>99)return 0;
  return major*100+minor;
}
static void clean_environment(void) {
  for(size_t i=0;environ[i];) {
    if(strncmp(environ[i],"DYLD_",5)==0 || strncmp(environ[i],"LD_",3)==0) {
      char* equal=strchr(environ[i],'=');
      if(equal){
        size_t n=(size_t)(equal-environ[i]); char name[1024];
        if(n>=sizeof(name))_exit(70);
        memcpy(name,environ[i],n);name[n]=0;
        if(unsetenv(name)!=0)_exit(70);
        continue;
      }
    }
    ++i;
  }
}
int main(int argc,char** argv) {
  if(signal(SIGALRM,SIG_DFL)==SIG_ERR)return 70;
  sigset_t timeout_signals;
  if(sigemptyset(&timeout_signals)!=0 || sigaddset(&timeout_signals,SIGALRM)!=0 ||
    sigprocmask(SIG_UNBLOCK,&timeout_signals,NULL)!=0)return 70;
  alarm(15);
  if(getuid()==0 || getuid()!=geteuid())return 70;
  clean_environment();
  enum colossus_mac_network_role role; int policy_fd;
  if(!arguments(argc,argv,&role,&policy_fd))return 71;
  char executable[PATH_MAX],raw[PATH_MAX]; uint32_t capacity=sizeof(raw);
  if(_NSGetExecutablePath(raw,&capacity)!=0 || !realpath(raw,executable))return 72;
  char* nested=strstr(executable,"/Contents/Frameworks/");
  if(!nested)return 72;
  char bundle[PATH_MAX]; size_t prefix=(size_t)(nested-executable);
  if(prefix>=sizeof(bundle))return 72; memcpy(bundle,executable,prefix);bundle[prefix]=0;
  char storage[32*1024+1]; size_t size;
  struct colossus_mac_network_binding binding;
  if(!resource(bundle,storage,sizeof(storage),&size) ||
    !colossus_mac_network_binding_parse(storage,size,&binding) || strcmp(bundle,binding.bundle_path))return 73;
  auditinfo_addr_t audit;
  if(getaudit_addr(&audit,sizeof(audit))!=0 || (uint32_t)audit.ai_asid!=binding.audit_session || getppid()<=1)return 74;
  char checked[PATH_MAX];
  const char* paths[]={binding.allocation_root,binding.bundle_path,binding.profile_root,binding.broker_root,binding.personal_home};
  for(size_t i=0;i<sizeof(paths)/sizeof(paths[0]);++i)if(!canonical(paths[i],checked))return 75;
  struct stat root;
  if(stat(binding.allocation_root,&root)!=0 || !S_ISDIR(root.st_mode) || root.st_uid!=geteuid() || (root.st_mode&0777)!=0700)return 75;
  char body[PATH_MAX]; int written=snprintf(body,sizeof(body),"%s Body",executable);
  if(written<0||(size_t)written>=sizeof(body)||!canonical(body,checked))return 76;
  struct stat image;
  if(lstat(body,&image)!=0 || !S_ISREG(image.st_mode) || image.st_uid!=geteuid() || image.st_nlink!=1 || (image.st_mode&0022))return 76;
  if(!discard_browser_policy(policy_fd))return 77;
  char* source=NULL;
  if(!colossus_mac_helper_body_policy(role,&binding,executable,body,getppid(),os_version(),&source))return 78;
  const char* parameters[]={NULL}; char* error=NULL;
  int applied=sandbox_init_with_parameters(source,0,parameters,&error);
  free(source);
  if(error)sandbox_free_error(error);
  if(applied!=0)return 79;
  int output=1;
  for(int i=1;i<argc;++i)if(strncmp(argv[i],"--seatbelt-client=",18))argv[output++]=argv[i];
  argv[0]=body; argv[output]=NULL;
  memset(storage,0,sizeof(storage));
  alarm(0);
  execve(body,argv,environ);
  return 80;
}
