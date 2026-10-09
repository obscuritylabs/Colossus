#include "host_internal.h"
#include "include/cef_application_mac.h"
#include "include/wrapper/cef_library_loader.h"
#import <Cocoa/Cocoa.h>

@interface ColossusCefApplication : NSApplication <CefAppProtocol> {
  BOOL handlingSendEvent_;
}
@end
@implementation ColossusCefApplication
- (BOOL)isHandlingSendEvent { return handlingSendEvent_; }
- (void)setHandlingSendEvent:(BOOL)value { handlingSendEvent_ = value; }
- (void)sendEvent:(NSEvent*)event {
  CefScopedSendingEvent sendingEvent;
  [super sendEvent:event];
}
@end

namespace colossus {
bool PlatformLoadLibrary() {
  // Main-process framework must be loaded dynamically for macOS sandboxing.
  // Helpers have a different entry that initializes their sandbox first.
  static CefScopedLibraryLoader loader;
  static bool loaded = loader.LoadInMain();
  return loaded;
}
bool PlatformEarlySetup() {
  if (![NSThread isMainThread]) return false;
  if (NSApp && ![NSApp conformsToProtocol:@protocol(CefAppProtocol)]) return false;
  if (!NSApp) [ColossusCefApplication sharedApplication];
  return [NSApp conformsToProtocol:@protocol(CefAppProtocol)];
}
int32_t MacBounds(CefWindowHandle handle, colossus_cef_bounds bounds) {
  NSView* view = (__bridge NSView*)handle;
  if (!view || ![NSThread isMainThread]) return COLOSSUS_CEF_CLOSED;
  auto converted = PlatformChildBounds(reinterpret_cast<uintptr_t>((__bridge void*)[view superview]), bounds);
  [view setFrame:NSMakeRect(converted.x, converted.y, converted.width, converted.height)];
  return COLOSSUS_CEF_OK;
}
int32_t MacVisible(CefWindowHandle handle, bool visible) {
  NSView* view = (__bridge NSView*)handle;
  if (!view || ![NSThread isMainThread]) return COLOSSUS_CEF_CLOSED;
  [view setHidden:!visible];
  if (!visible && [[view window] firstResponder] == view) [[view window] makeFirstResponder:nil];
  return COLOSSUS_CEF_OK;
}
colossus_cef_bounds PlatformChildBounds(uintptr_t handle, colossus_cef_bounds bounds) {
  NSView* parent = (__bridge NSView*)reinterpret_cast<void*>(handle);
  if (parent && ![parent isFlipped])
    bounds.y = static_cast<int32_t>(NSHeight([parent bounds])) - bounds.y - bounds.height;
  return bounds;
}
}
