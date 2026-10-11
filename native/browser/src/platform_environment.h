#ifndef COLOSSUS_CEF_PLATFORM_ENVIRONMENT_H_
#define COLOSSUS_CEF_PLATFORM_ENVIRONMENT_H_

namespace colossus {
// Dedicated macOS startup verifies initial inherited owned paths only; it never
// mutates process-wide environment or selects an ambient cache/temp service.
bool PlatformDedicatedEnvironment(const char* root_cache_path);
}

#endif
