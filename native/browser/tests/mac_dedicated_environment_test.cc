// No CEF/AppKit application/Keychain initialization. This isolated process
// changes its own test environment before any native/helper thread exists.
#include "platform_environment.h"
#include <cstdio>
#include <cstdlib>
#include <string>
#include <sys/stat.h>
#include <unistd.h>

int main() {
  char root[] = "/private/tmp/colossus-owned-environment-XXXXXX";
  if (!mkdtemp(root)) return 1;
  const std::string temporary = std::string(root) + "/tmp";
  if (mkdir(temporary.c_str(), 0700)) return 2;
  bool valid = setenv("TMPDIR", temporary.c_str(), 1) == 0 &&
    unsetenv("MAC_CHROMIUM_TMPDIR") == 0 && !colossus::PlatformDedicatedEnvironment(root);
  valid = valid && setenv("MAC_CHROMIUM_TMPDIR", "/private/tmp", 1) == 0 &&
    !colossus::PlatformDedicatedEnvironment(root);
  valid = valid && setenv("MAC_CHROMIUM_TMPDIR", temporary.c_str(), 1) == 0 &&
    colossus::PlatformDedicatedEnvironment(root);
  valid = valid && chmod(temporary.c_str(), 0777) == 0 &&
    !colossus::PlatformDedicatedEnvironment(root) && chmod(temporary.c_str(), 0700) == 0;
  valid = valid && setenv("TMPDIR", "/private/tmp", 1) == 0 &&
    !colossus::PlatformDedicatedEnvironment(root);
  const bool cleaned = rmdir(temporary.c_str()) == 0 && rmdir(root) == 0;
  if (valid && cleaned)
    std::puts("PASS initial owned Chromium temporary environment; mismatches/private-mode failures denied; CEF/Keychain=false");
  return valid && cleaned ? 0 : 3;
}
