// AppKit registration only under the actual inherited main envelope. No window,
// CEF, Keychain, audit port or delegated application launch occurs in this test.
#import <Cocoa/Cocoa.h>
#include <cstdio>
#include <unistd.h>

extern "C" int sandbox_check(pid_t, const char*, int, ...);

extern "C" int colossus_appkit_fixture_initialize(void) {
  // libsandbox's fixed global-name filter, validated by contrasting exact
  // permitted registration endpoints with the otherwise denied services.
  constexpr int global_name = 2;
  const char* allowed[] = {"com.apple.coreservices.launchservicesd",
                         "com.apple.CoreServices.coreservicesd"};
  for (const auto* service : allowed)
    if (sandbox_check(getpid(), "mach-lookup", global_name, service) != 0) return 40;
  const char* denied[] = {"com.apple.SecurityServer", "com.apple.securityd",
    "com.apple.pasteboard.1", "com.apple.distributed_notifications",
    "com.apple.cfprefsd.agent", "com.apple.tccd"};
  for (const auto* service : denied)
    if (sandbox_check(getpid(), "mach-lookup", global_name, service) != 1) return 41;
  if (sandbox_check(getpid(), "lsopen", 0) != 1 ||
      sandbox_check(getpid(), "appleevent-send", 0) != 1) return 42;
  @autoreleasepool {
    if (![NSThread isMainThread] || ![NSApplication sharedApplication]) return 43;
    if (![NSApp setActivationPolicy:NSApplicationActivationPolicyProhibited]) return 44;
    [NSApp finishLaunching];
    if ([NSApp activationPolicy] != NSApplicationActivationPolicyProhibited ||
        [[NSApp windows] count] != 0 || sandbox_check(getpid(), nullptr, 0) != 1)
      return 45;
  }
  std::puts("PASS AppKit registration with exact services; kernel lsopen/AppleEvent checks denied; no windows/CEF/Keychain");
  return 0;
}
