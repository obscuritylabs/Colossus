// The fixed CEF body runs only under its front's inherited composite policy.
#include "include/cef_app.h"
#include "include/cef_sandbox_mac.h"
#include "include/wrapper/cef_library_loader.h"
#include <cstring>
#include <cstdio>
#include <unistd.h>
extern "C" int sandbox_check(pid_t, const char*, int, ...);
static void diagnostic(int stage,int status) {
#if defined(COLOSSUS_CEF_MAC_SYNTHETIC_PROFILE_DIAGNOSTIC)
  std::fprintf(stderr,"native helper body pid=%d stage=%d status=%d\n",getpid(),stage,status);
#else
  (void)stage; (void)status;
#endif
}
static int failure(int status) { diagnostic(status,1); return status; }

int main(int argc,char** argv) {
  diagnostic(1,0);
  if(argc<2 || sandbox_check(getpid(),nullptr,0)!=1)return failure(81);
  for(int i=1;i<argc;++i)
    if(std::strncmp(argv[i],"--seatbelt-client=",18)==0 ||
       std::strcmp(argv[i],"--no-sandbox")==0)return failure(81);
  // No seatbelt-client remains: stock context management retains compatibility
  // without attempting to apply a second policy over the inherited composite.
  CefScopedSandboxContext context;
  if(!context.Initialize(argc,argv))return failure(82);
  diagnostic(2,0);
  CefScopedLibraryLoader library;
  if(!library.LoadInHelper())return failure(83);
  diagnostic(3,0);
  int result=CefExecuteProcess(CefMainArgs(argc,argv),nullptr,nullptr);
  diagnostic(4,result);
  return result;
}
