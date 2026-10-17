/* Pure source-builder contract test: no sandbox/audit/process effects. */
#include "network_envelope.h"
#include <assert.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

#define BUNDLE "/private/tmp/owner/Browser.app"
#define BODY BUNDLE "/Contents/MacOS/colossus-native-browser-host Body"
#define FRONT(s) BUNDLE "/Contents/Frameworks/Colossus Browser Helper" s ".app/Contents/MacOS/Colossus Browser Helper" s
static int balanced(const char* source) {
  int depth=0,quoted=0,comment=0,escape=0;
  for(const char* c=source;*c;++c) {
    if(comment){if(*c=='\n')comment=0;continue;}
    if(quoted){if(escape)escape=0;else if(*c=='\\')escape=1;else if(*c=='"')quoted=0;continue;}
    if(*c==';'){comment=1;continue;}
    if(*c=='"'){quoted=1;continue;}
    if(*c=='(')++depth;
    if(*c==')'&&--depth<0)return 0;
  }
  return depth==0&&!quoted;
}
int main(void) {
  struct colossus_mac_network_binding b={42,54321,"/private/tmp/owner",BUNDLE,
    "com.colossus.browser.fixture","/private/tmp/owner/profile","/private/tmp/broker","/Users/fixture"};
  char* source=NULL;
  assert(colossus_mac_helper_body_policy(COLOSSUS_MAC_ROLE_RENDERER,&b,FRONT(" (Renderer)"),
    FRONT(" (Renderer)") " Body",123,2600,&source));
  assert(strstr(source,"; Copyright 2017 The Chromium Authors"));
  assert(balanced(source));
  assert(strstr(source,"; Put the denials first."));
#if defined(COLOSSUS_EXPECT_SYNTHETIC_PROFILE)
  assert(strstr(source,"(if (string=? key \"DISABLE_SANDBOX_DENIAL_LOGGING\") \"FALSE\""));
#else
  assert(strstr(source,"(if (string=? key \"DISABLE_SANDBOX_DENIAL_LOGGING\") \"TRUE\""));
#endif
  assert(strstr(source,"(deny network*)"));
  assert(strstr(source,"(allow file-read-data (literal \"/\"))"));
  assert(!strstr(source,"(allow file-read-data (subpath \"/\"))"));
  assert(!strstr(source,"(allow file-read* (subpath \"/\"))"));
  assert(!strstr(source,"localhost:54321"));
  assert(!strstr(source,"(with no-sandbox)"));
  assert(strstr(source,"(allow process-exec (literal \"" FRONT(" (Renderer)") " Body\"))"));
  assert(strlen(source)<128*1024);free(source);
  assert(colossus_mac_helper_policy(COLOSSUS_MAC_ROLE_NETWORK,&b,FRONT(""),123,2600,&source));
  const char* original=strstr(source,"; Network socket access.");
  const char* restriction=strstr(source,"; Colossus mandatory native envelope");
  assert(original && restriction && original<restriction);
  assert(strstr(restriction,"(remote tcp \"localhost:54321\")"));
  assert(!strstr(source,"(remote tcp)"));
  assert(!strstr(source,"(remote udp)"));
  assert(!strstr(source,"(allow network-bind"));
  assert(!strstr(source,"(allow system-socket"));
  assert(!strstr(restriction,"com.apple.SecurityServer"));free(source);
  for(int role=0;role<=COLOSSUS_MAC_ROLE_WEBNN;++role) {
    if(role==COLOSSUS_MAC_ROLE_SCREEN_AI || role==COLOSSUS_MAC_ROLE_SPEECH_RECOGNITION ||
      role==COLOSSUS_MAC_ROLE_ON_DEVICE_TRANSLATION || role==COLOSSUS_MAC_ROLE_WEBNN) {
      assert(!colossus_mac_helper_policy((enum colossus_mac_network_role)role,&b,FRONT(""),123,2600,&source));
      continue;
    }
    const char* e=role==COLOSSUS_MAC_ROLE_RENDERER?FRONT(" (Renderer)"):
      role==COLOSSUS_MAC_ROLE_GPU?FRONT(" (GPU)"):FRONT("");
    assert(colossus_mac_helper_policy((enum colossus_mac_network_role)role,&b,e,123,2600,&source));
    assert(balanced(source));
    assert(!strstr(source,"(allow network-bind"));
    const char* constrained=strstr(source,"; Colossus mandatory native envelope");
    assert(constrained && !strstr(constrained,"(global-name \"com.apple.bsd.dirhelper\")"));free(source);
  }
  assert(!colossus_mac_helper_policy(COLOSSUS_MAC_ROLE_NETWORK,&b,FRONT(" (Renderer)"),123,2600,&source));
  assert(!source);
  assert(!colossus_mac_helper_body_policy(COLOSSUS_MAC_ROLE_RENDERER,&b,FRONT(" (Renderer)"),"/bin/sh",123,2600,&source));
  assert(!colossus_mac_helper_policy((enum colossus_mac_network_role)99,&b,FRONT(""),123,2600,&source));
  b.proxy_port=0;assert(!colossus_mac_helper_policy(COLOSSUS_MAC_ROLE_NETWORK,&b,FRONT(""),123,2600,&source));b.proxy_port=54321;
  b.profile_root="/private/tmp/owner/../outside";
  assert(!colossus_mac_helper_policy(COLOSSUS_MAC_ROLE_NETWORK,&b,FRONT(""),123,2600,&source));
  b.profile_root="/private/tmp/owner/profile";
  const char* fronts[]={FRONT(""),FRONT(" (Renderer)"),FRONT(" (GPU)"),FRONT(" (Plugin)"),FRONT(" (Alerts)")};
  assert(colossus_mac_main_network_policy(&b,fronts,5,&source));
  assert(strstr(source,"(with no-sandbox)"));assert(strstr(source,"(deny process-exec*)"));
  assert(strstr(source,"(allow process-exec (literal \"" BODY "\"))"));
  assert(!strstr(source,"(allow process-exec (literal \"" BODY "\") (with no-sandbox))"));
  assert(!strstr(source,"(allow process-exec (subpath"));free(source);
  assert(colossus_mac_main_network_policy(&b,fronts,5,&source));
  assert(strstr(source,"(allow file-read-metadata (literal \"/private\"))"));
  assert(strstr(source,"(allow file-read-metadata (literal \"/private/tmp\"))"));
  assert(!strstr(source,"(allow file-read-data"));
  assert(!strstr(source,"(allow file-read*"));
  assert(strstr(source,"(allow network-bind (local unix-socket (subpath \"/private/tmp/owner/profile/tmp\")))"));
  assert(!strstr(source,"(allow network-bind (local tcp"));
  assert(!strstr(source,"(allow network-bind (local ip"));
  assert(!strstr(source,"(allow network-inbound"));
  assert(!strstr(source,"(allow network-outbound (remote unix-socket"));
#if defined(COLOSSUS_EXPECT_SYNTHETIC_PROFILE)
  assert(strstr(source,"(deny file-read-data file-read-xattr (require-all (require-any (subpath \"/private/var/folders\") (subpath \"/var/folders\")) (require-not (subpath \"/private/tmp/owner\"))))"));
  assert(strstr(source,"(allow mach-lookup (global-name \"com.apple.bsd.dirhelper\"))"));
#else
  assert(!strstr(source,"(subpath \"/private/var/folders\")"));
  assert(!strstr(source,"(global-name \"com.apple.bsd.dirhelper\")"));
#endif
  assert(strstr(source,"(global-name \"com.apple.coreservices.launchservicesd\")"));
  assert(strstr(source,"(global-name \"com.apple.CoreServices.coreservicesd\")"));
  assert(strstr(source,"(deny lsopen)"));
  assert(strstr(source,"(deny appleevent-send)"));
  assert(!strstr(source,"(global-name \"com.apple.SecurityServer\")"));free(source);
  fronts[4]="/bin/sh";assert(!colossus_mac_main_network_policy(&b,fronts,5,&source));
  char input[]="COLOSSUS_MAC_NETWORK_ENVELOPE_V1\n42\n54321\n/private/tmp/owner\n" BUNDLE "\ncom.colossus.browser.fixture\n/private/tmp/owner/profile\n/private/tmp/broker\n/Users/fixture\n";
  assert(colossus_mac_network_binding_parse(input,sizeof(input)-1,&b));assert(b.audit_session==42&&b.proxy_port==54321);
  char bad[]="COLOSSUS_MAC_NETWORK_ENVELOPE_V1\n42\n65536\n/private/tmp/owner\n" BUNDLE "\ncom.colossus.browser.fixture\n/private/tmp/owner/profile\n/private/tmp/broker\n/Users/fixture\n";
  assert(!colossus_mac_network_binding_parse(bad,sizeof(bad)-1,&b));
  puts("macOS network policy source contracts passed");
}
