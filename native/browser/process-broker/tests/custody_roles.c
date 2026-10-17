/* Pure actual pinned role-builder test; no sandbox/process/Security/CEF APIs. */
#include "../../mac/network_envelope.h"
#include "../custody_policy.h"
#include "../policy_transition.h"
#include <assert.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
int main(void) {
  const struct colossus_mac_network_binding binding={42,12345,"/private/allocation","/private/allocation/Browser.app","com.obscuritylabs.fixture","/private/allocation/profile","/private/custody","/Users/fixture"};
  static const char* suffixes[]={""," (Alerts)"," (GPU)"," (Plugin)"," (Renderer)"};
  char paths[5][PATH_MAX];const char* fronts[5];
  for(size_t i=0;i<5;++i){int n=snprintf(paths[i],sizeof(paths[i]),"%s/Contents/Frameworks/Colossus Browser Helper%s.app/Contents/MacOS/Colossus Browser Helper%s",binding.bundle_path,suffixes[i],suffixes[i]);assert(n>0&&n<PATH_MAX);fronts[i]=paths[i];}
  static const enum colossus_mac_network_role roles[]={COLOSSUS_MAC_ROLE_RENDERER,COLOSSUS_MAC_ROLE_GPU,COLOSSUS_MAC_ROLE_NETWORK,COLOSSUS_MAC_ROLE_UTILITY};
  static const size_t names[]={4,2,0,0};
  for(size_t role=0;role<5;++role) {
    char* source=NULL;char* no_process=NULL;char* final=NULL;
    assert(role==0?colossus_mac_main_network_policy(&binding,fronts,5,&source):colossus_mac_helper_policy(roles[role-1],&binding,fronts[names[role-1]],42,2600,&source));
    assert(colossus_owned_process_policy(source,strlen(source),&no_process));
    assert(colossus_owned_custody_policy(no_process,strlen(no_process),binding.broker_root,&final));
    assert(strstr(final,"(require-not (subpath \"/private/custody\"))"));
    assert(!strstr(final,"(allow file-issue-extension"));assert(!strstr(final,"(allow process-exec"));
    assert(strlen(final)<=128*1024);free(final);free(no_process);free(source);
  }
}
