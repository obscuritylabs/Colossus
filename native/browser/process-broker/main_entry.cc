// Fixed no-exec Main registration. CEF is loaded only after Rust bootstrap has
// consumed its private channels and closed unrelated descriptors.
#include <array>
#include <dlfcn.h>
#include <fcntl.h>
#include <limits.h>
#include <mach-o/dyld.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

namespace {
enum class Phase { Empty, Prepared, Installed, Failed };
Phase phase=Phase::Empty;
std::array<uint8_t,16> enrollment{};
uint32_t browser=0;
}

extern "C" bool colossus_cef_owned_process_install(const uint8_t* nonce,
                                                    uint32_t browser_pid) {
  if(phase!=Phase::Empty||!nonce||browser_pid!=(uint32_t)getpid())return false;
  phase=Phase::Failed;
  bool nonzero=false;for(size_t i=0;i<enrollment.size();++i)nonzero|=nonce[i]!=0;
  struct stat channel;
  if(!nonzero||fstat(7,&channel)||!S_ISSOCK(channel.st_mode)||fcntl(7,F_SETFD,FD_CLOEXEC))return false;
  memcpy(enrollment.data(),nonce,enrollment.size());browser=browser_pid;
  phase=Phase::Prepared;return true;
}

namespace colossus {
bool OwnedProcessInstallLoadedFramework() {
  if(phase==Phase::Installed)return true;
  if(phase!=Phase::Prepared)return false;
  phase=Phase::Failed;
  char raw[PATH_MAX],front[PATH_MAX],framework[PATH_MAX];uint32_t capacity=sizeof(raw);
  if(_NSGetExecutablePath(raw,&capacity)||!realpath(raw,front))return false;
  const char* split=strstr(front,"/Contents/MacOS/");if(!split)return false;
  int length=snprintf(framework,sizeof(framework),"%.*s/Contents/Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework",(int)(split-front),front);
  if(length<=0||length>=(int)sizeof(framework))return false;
  void* handle=dlopen(framework,RTLD_NOLOAD|RTLD_LAZY|RTLD_LOCAL|RTLD_FIRST);if(!handle)return false;
  using Install=bool(*)(int,const uint8_t*,pid_t);
  auto install=reinterpret_cast<Install>(dlsym(handle,"colossus_owned_process_install"));
  const bool result=install&&install(7,enrollment.data(),(pid_t)browser);
  dlclose(handle);memset(enrollment.data(),0,enrollment.size());
  if(result)phase=Phase::Installed;
  return result;
}
}
