// Private no-exec helper body library. This source is not part of current CEF
// staging. Link its private install symbol to the reviewed patched component.
#include "include/cef_app.h"
#include "include/cef_sandbox_mac.h"
#include "include/wrapper/cef_library_loader.h"
#include <cstring>
#include <cstdio>
#include <dlfcn.h>
#include <limits.h>
#include <mach-o/dyld.h>
#include <stdlib.h>
#include <stdint.h>
#include <unistd.h>
extern "C" int sandbox_check(pid_t,const char*,int,...);

static bool install_logical_parent(pid_t browser_pid) {
  char raw[PATH_MAX],front[PATH_MAX],framework[PATH_MAX];uint32_t capacity=sizeof(raw);
  if(_NSGetExecutablePath(raw,&capacity)!=0||!realpath(raw,front))return false;
  const char* split=std::strstr(front,"/Contents/Frameworks/");if(!split)return false;
  const int length=std::snprintf(framework,sizeof(framework),"%.*s/Contents/Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework",static_cast<int>(split-front),front);
  if(length<=0||length>=static_cast<int>(sizeof(framework)))return false;
  // CEF loads RTLD_LOCAL|RTLD_FIRST. Resolve the private symbol only from that
  // exact already-loaded framework, never from RTLD_DEFAULT or another image.
  void* handle=dlopen(framework,RTLD_NOLOAD|RTLD_LAZY|RTLD_LOCAL|RTLD_FIRST);
  if(!handle)return false;
  using Install=bool(*)(pid_t);
  auto install=reinterpret_cast<Install>(dlsym(handle,"colossus_owned_helper_install"));
  const bool result=install&&install(browser_pid);dlclose(handle);return result;
}

extern "C" __attribute__((visibility("default")))
int colossus_owned_body_main(int argc,char** argv,uint32_t browser_pid,
                            const uint8_t* nonce,uint8_t role) {
  if(argc<2||argc>129||!nonce||role<1||role>4||browser_pid<=1||
     sandbox_check(getpid(),nullptr,0)!=1)return 81;
  for(int i=1;i<argc;++i)
    if(std::strncmp(argv[i],"--seatbelt-client=",18)==0||
       std::strcmp(argv[i],"--no-sandbox")==0)return 81;
  CefScopedSandboxContext context;
  if(!context.Initialize(argc,argv))return 82;
  CefScopedLibraryLoader library;
  if(!library.LoadInHelper())return 83;
  if(!install_logical_parent(static_cast<pid_t>(browser_pid)))return 84;
  return CefExecuteProcess(CefMainArgs(argc,argv),nullptr,nullptr);
}
