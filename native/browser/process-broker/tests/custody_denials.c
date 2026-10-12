/* Root-owned opt-in native policy/application diagnostic. This applies the
 * actual pinned five role builders plus both local immutable transformers, but
 * DOES NOT launch current owned fronts/CEF, issue custody acceptance reports,
 * open any Keychain, receive Mach rights or change global Security state. */
#include "../../mac/network_envelope.h"
#include "../custody_policy.h"
#include "../policy_transition.h"
#include <sys/stat.h>
#include <sys/wait.h>
#include <fcntl.h>
#include <unistd.h>
#include <signal.h>
#include <limits.h>
#include <errno.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <time.h>
extern int sandbox_init_with_parameters(const char*,unsigned long long,const char*const*,char**);
extern int sandbox_check(pid_t,const char*,int,...);
extern char* sandbox_extension_issue_file(const char*,const char*,unsigned int);
static int denied(int result) { return result<0&&(errno==EPERM||errno==EACCES); }
static int child_probe(const char* source,const char* marker,const char* absent,const char* alias) {
  pid_t child=fork();if(child<0)return 0;
  if(child==0) {
    char* error=NULL;const char* parameters[]={NULL};
    if(sandbox_init_with_parameters(source,0,parameters,&error)||sandbox_check(getpid(),NULL,0)!=1)_exit(10);
    errno=0;int fd=open(marker,O_RDONLY|O_NOFOLLOW);if(!denied(fd)){if(fd>=0)close(fd);_exit(11);}
    errno=0;fd=open(marker,O_WRONLY|O_NOFOLLOW);if(!denied(fd)){if(fd>=0)close(fd);_exit(12);}
    errno=0;fd=open(absent,O_WRONLY|O_CREAT|O_EXCL,0600);if(!denied(fd)){if(fd>=0)close(fd);_exit(13);}
    errno=0;if(!denied(rename(marker,absent)))_exit(14);
    errno=0;if(!denied(unlink(marker)))_exit(15);
    errno=0;if(!denied(link(marker,alias)))_exit(16);
    errno=0;char* extension=sandbox_extension_issue_file("com.apple.app-sandbox.read-write",marker,0);
    if(extension||!(errno==EPERM||errno==EACCES))_exit(17);
    _exit(0);
  }
  int status=0;for(unsigned i=0;i<1000;++i){pid_t result=waitpid(child,&status,WNOHANG);if(result==child)return WIFEXITED(status)&&WEXITSTATUS(status)==0;
    if(result<0){if(errno==EINTR)continue;return 0;}struct timespec delay={0,10000000};nanosleep(&delay,NULL);}
  kill(child,SIGKILL);while(waitpid(child,&status,0)<0&&errno==EINTR){}return 0;
}
int main(void) {
  char allocation[]="/private/tmp/colossus-custody-role-allocation-XXXXXX",custody[]="/private/tmp/colossus-custody-role-files-XXXXXX";
  if(!mkdtemp(allocation)||!mkdtemp(custody))return 1;
  char app[PATH_MAX],profile[PATH_MAX],marker[PATH_MAX],absent[PATH_MAX],alias[PATH_MAX];
  if(snprintf(app,sizeof(app),"%s/Browser.app",allocation)<=0||snprintf(profile,sizeof(profile),"%s/profile",allocation)<=0||
     snprintf(marker,sizeof(marker),"%s/fixture-marker",custody)<=0||snprintf(absent,sizeof(absent),"%s/denied-create",custody)<=0||
     snprintf(alias,sizeof(alias),"%s/denied-link",profile)<=0||mkdir(profile,0700))return 2;
  int file=open(marker,O_WRONLY|O_CREAT|O_EXCL|O_NOFOLLOW,0600);if(file<0||write(file,"public fixture",14)!=14||close(file))return 2;
  const struct colossus_mac_network_binding binding={42,12345,allocation,app,"com.obscuritylabs.fixture",profile,custody,"/Users/fixture"};
  static const char* suffixes[]={""," (Alerts)"," (GPU)"," (Plugin)"," (Renderer)"};char paths[5][PATH_MAX];const char* fronts[5];
  for(size_t i=0;i<5;++i){int n=snprintf(paths[i],sizeof(paths[i]),"%s/Contents/Frameworks/Colossus Browser Helper%s.app/Contents/MacOS/Colossus Browser Helper%s",app,suffixes[i],suffixes[i]);if(n<=0||n>=PATH_MAX)return 3;fronts[i]=paths[i];}
  static const enum colossus_mac_network_role roles[]={COLOSSUS_MAC_ROLE_RENDERER,COLOSSUS_MAC_ROLE_GPU,COLOSSUS_MAC_ROLE_NETWORK,COLOSSUS_MAC_ROLE_UTILITY};static const size_t names[]={4,2,0,0};
  for(size_t role=0;role<5;++role){char* original=NULL;char* no_process=NULL;char* final=NULL;
    int built=role==0?colossus_mac_main_network_policy(&binding,fronts,5,&original):colossus_mac_helper_policy(roles[role-1],&binding,fronts[names[role-1]],getpid(),2600,&original);
    if(!built||!colossus_owned_process_policy(original,strlen(original),&no_process)||!colossus_owned_custody_policy(no_process,strlen(no_process),custody,&final))return 4;
    int passed=child_probe(final,marker,absent,alias);free(final);free(no_process);free(original);
    if(!passed){fprintf(stderr,"custody role diagnostic failed: role=%zu\n",role);return 5;}}
  if(unlink(marker)||rmdir(profile)||rmdir(custody)||rmdir(allocation))return 6;
  puts("Five fixed role policy applications denied custody file operations and extension issuance; current owned-front/package acceptance remains separate.");return 0;
}
