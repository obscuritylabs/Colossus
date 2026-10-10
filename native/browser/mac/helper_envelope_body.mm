// The fixed CEF body runs only under its front's inherited composite policy.
#include "include/cef_app.h"
#include "include/cef_sandbox_mac.h"
#include "include/wrapper/cef_library_loader.h"
#include <cstring>
#include <unistd.h>
extern "C" int sandbox_check(pid_t, const char*, int, ...);

int main(int argc,char** argv) {
  if(argc<2 || sandbox_check(getpid(),nullptr,0)!=1)return 81;
  for(int i=1;i<argc;++i)
    if(std::strncmp(argv[i],"--seatbelt-client=",18)==0 ||
       std::strcmp(argv[i],"--no-sandbox")==0)return 81;
  // No seatbelt-client remains: stock context management retains compatibility
  // without attempting to apply a second policy over the inherited composite.
  CefScopedSandboxContext context;
  if(!context.Initialize(argc,argv))return 82;
  CefScopedLibraryLoader library;
  if(!library.LoadInHelper())return 83;
  return CefExecuteProcess(CefMainArgs(argc,argv),nullptr,nullptr);
}
