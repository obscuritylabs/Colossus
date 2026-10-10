#include "host_internal.h"
#include "include/cef_application_mac.h"
#include "include/wrapper/cef_library_loader.h"
#import <Cocoa/Cocoa.h>
#import <objc/runtime.h>
#include <algorithm>
#include <cstdio>
#include <vector>

namespace {
using SendEvent = void (*)(id, SEL, NSEvent*);

SendEvent TaoSendEvent() {
  // Tao registers its class when Tauri builds the event loop, after early CEF
  // setup has already created NSApp. Forward its implementation instead of
  // replacing its delegate or losing its Cmd-key-up/device-event handling.
  // The pinned Tao class has no ivars and directly extends NSApplication, so
  // its implementation's dynamic superclass dispatch is valid for our class.
  Class tao = NSClassFromString(@"TaoApp");
  if (!tao || class_getSuperclass(tao) != [NSApplication class] ||
      class_getInstanceSize(tao) != class_getInstanceSize([NSApplication class]))
    return nullptr;
  return reinterpret_cast<SendEvent>(class_getMethodImplementation(tao, @selector(sendEvent:)));
}

bool TaoEventLoopInstalled() {
  Class delegate = NSClassFromString(@"TaoAppDelegateParent");
  return TaoSendEvent() && delegate && [(NSObject*)[NSApp delegate] isKindOfClass:delegate];
}
}

@interface ColossusCefApplication : NSApplication <CefAppProtocol> {
  BOOL handlingSendEvent_;
}
@end
@implementation ColossusCefApplication
- (BOOL)isHandlingSendEvent { return handlingSendEvent_; }
- (void)setHandlingSendEvent:(BOOL)value { handlingSendEvent_ = value; }
- (void)sendEvent:(NSEvent*)event {
  CefScopedSendingEvent sendingEvent;
  if (auto send = TaoSendEvent()) send(self, _cmd, event);
  else [super sendEvent:event];
}
- (void)terminate:(id)sender {
  std::fprintf(stderr, "native AppKit terminate intercepted (initialized=%d)\n",
               colossus::initialized);
  if (colossus::initialized) {
    // Cmd-Q and Dock Quit otherwise enter Tao's applicationWillTerminate path
    // directly, skipping Tauri ExitRequested and native browser close waiting.
    // The trusted host callback requests Tauri exit while AppKit keeps pumping.
    colossus::Emit(0, 0, COLOSSUS_CEF_APPLICATION_QUIT);
    return;
  }
  [super terminate:sender];
}
@end

extern "C" int32_t colossus_cef_standalone_platform_pump() {
  if (!colossus::initialized) return COLOSSUS_CEF_UNAVAILABLE;
  if (![NSThread isMainThread] ||
      std::this_thread::get_id() != colossus::owning_thread)
    return COLOSSUS_CEF_WRONG_THREAD;
  if (!NSApp || ![NSApp conformsToProtocol:@protocol(CefAppProtocol)] ||
      TaoEventLoopInstalled())
    return COLOSSUS_CEF_UNAVAILABLE;
  @autoreleasepool {
    static bool launched = false;
    if (!launched) {
      // This dedicated OSR process has no window or desktop delegate. Its
      // authenticated presenter lives in the Desktop process. Do not register
      // an additional Dock application or replace a Tauri application delegate.
      [NSApp setActivationPolicy:NSApplicationActivationPolicyProhibited];
      [NSApp finishLaunching];
      launched = true;
    }
    for (unsigned int count = 0; count < 32; ++count) {
      NSEvent* event = [NSApp nextEventMatchingMask:NSEventMaskAny
                         untilDate:[NSDate distantPast]
                            inMode:NSDefaultRunLoopMode dequeue:YES];
      if (!event) break;
      [NSApp sendEvent:event];
    }
    [NSApp updateWindows];
  }
  return COLOSSUS_CEF_OK;
}

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
void PlatformEventLoopDiagnostics() {
  static bool reported = false;
  if (reported) return;
  reported = true;
  const char* application = [NSStringFromClass([NSApp class]) UTF8String];
  const char* delegate = [NSStringFromClass([[NSApp delegate] class]) UTF8String];
  const bool quit_hook = class_getMethodImplementation([NSApp class], @selector(terminate:)) ==
      class_getMethodImplementation([ColossusCefApplication class], @selector(terminate:));
  std::fprintf(stderr,
      "native AppKit event loop: application=%s cef_protocol=%d tao_delegate=%d "
      "quit_hook=%d delegate=%s\n", application ? application : "none",
      [NSApp conformsToProtocol:@protocol(CefAppProtocol)], TaoEventLoopInstalled(),
      quit_hook, delegate ? delegate : "none");
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
  if (!visible) {
    NSResponder* responder = [[view window] firstResponder];
    if ([responder isKindOfClass:[NSView class]] &&
        [(NSView*)responder isDescendantOf:view])
      [[view window] makeFirstResponder:nil];
  }
  return COLOSSUS_CEF_OK;
}
bool PlatformCloseChild(CefWindowHandle handle) {
  NSView* view = (__bridge NSView*)handle;
  if (!view || ![NSThread isMainThread]) return false;
  // CEF's default Alloy close sends performClose: to this child's Tauri
  // window. Instead release just the owned CefBrowserHostView. Its dealloc
  // notifies CEF WindowDestroyed, which settles OnBeforeClose and renderer
  // teardown. Defer removal until DoClose has returned to avoid reentrant
  // destruction inside CEF's close-state transition. Foundation retains the
  // selector target until delivery, then releases that final temporary owner.
  [view performSelectorOnMainThread:@selector(removeFromSuperview)
                        withObject:nil waitUntilDone:NO];
  return true;
}
int32_t PlatformAcceptanceActivate(CefWindowHandle handle) {
  NSView* view = (__bridge NSView*)handle;
  if (!view || ![NSThread isMainThread]) return COLOSSUS_CEF_WRONG_THREAD;
  NSWindow* window = [view window];
  if (!window || !TaoEventLoopInstalled()) return COLOSSUS_CEF_CLOSED;
  // Direct development launches can be registered but inactive on recent macOS.
  // Request activation through the running application's OS identity after
  // Tauri setup, instead of changing any production visibility/focus policy.
  NSRunningApplication* application = [NSRunningApplication currentApplication];
  [application activateWithOptions:NSApplicationActivateAllWindows];
  if (@available(macOS 14.0, *)) [NSApp activate];
  else [NSApp activateIgnoringOtherApps:YES];
  [window makeKeyAndOrderFront:nil];
  // AppKit applies activation asynchronously. The test must still observe
  // isActive/isKeyWindow, so issuing the request alone never passes acceptance.
  return COLOSSUS_CEF_OK;
}
int32_t PlatformAcceptanceTerminate(CefWindowHandle handle) {
  NSView* view = (__bridge NSView*)handle;
  if (!view || ![NSThread isMainThread]) return COLOSSUS_CEF_WRONG_THREAD;
  if (![view window] || !TaoEventLoopInstalled()) return COLOSSUS_CEF_CLOSED;
  [NSApp terminate:nil];
  return COLOSSUS_CEF_OK;
}
colossus_cef_bounds PlatformChildBounds(uintptr_t handle, colossus_cef_bounds bounds) {
  NSView* parent = (__bridge NSView*)reinterpret_cast<void*>(handle);
  if (parent && ![parent isFlipped])
    bounds.y = static_cast<int32_t>(NSHeight([parent bounds])) - bounds.y - bounds.height;
  return bounds;
}

bool PlatformAcceptanceEvidence(CefWindowHandle handle, const void* result,
                                size_t size, std::string* evidence) {
  if (![NSThread isMainThread] || !evidence || !result ||
      size > COLOSSUS_CEF_MAX_PROTOCOL_BYTES) return false;
  NSView* view = (__bridge NSView*)handle;
  if (!view || ![view superview]) return false;
  auto parsed = CefParseJSON(result, size, JSON_PARSER_RFC);
  if (!parsed || parsed->GetType() != VTYPE_DICTIONARY) return false;
  auto screenshot = parsed->GetDictionary()->GetString("data").ToString();
  auto png = CefBase64Decode(screenshot);
  if (!png || png->GetSize() > COLOSSUS_CEF_MAX_PROTOCOL_BYTES) return false;
  std::vector<uint8_t> bytes(png->GetSize());
  png->GetData(bytes.data(), bytes.size(), 0);
  NSData* data = [NSData dataWithBytes:bytes.data() length:bytes.size()];
  NSBitmapImageRep* bitmap = [NSBitmapImageRep imageRepWithData:data];
  if (!bitmap || [bitmap pixelsWide] <= 0 || [bitmap pixelsHigh] <= 0 ||
      uint64_t([bitmap pixelsWide]) * uint64_t([bitmap pixelsHigh]) > 16 * 1024 * 1024)
    return false;
  int magenta = 0, green = 0;
  const NSInteger samples = [bitmap samplesPerPixel];
  const bool alpha_first = ([bitmap bitmapFormat] & NSBitmapFormatAlphaFirst) != 0;
  const NSInteger red = alpha_first ? 1 : 0;
  const bool rgb8 = [bitmap bitsPerSample] == 8 && samples >= 3 && samples <= 4;
  // Bound native color inspection independently of the PNG dimensions.
  const NSInteger stride_x = std::max<NSInteger>(1, [bitmap pixelsWide] / 512);
  const NSInteger stride_y = std::max<NSInteger>(1, [bitmap pixelsHigh] / 512);
  for (NSInteger y = 0; y < [bitmap pixelsHigh]; y += stride_y) {
    for (NSInteger x = 0; x < [bitmap pixelsWide]; x += stride_x) {
      if (!rgb8) continue;
      // The fixed CSS fixture is color-managed by Chromium. A wide-gamut Mac
      // produced magenta [234,51,247] and green [117,251,76], so exact sRGB
      // bytes are not an invariant of native capture. Require bright, strongly
      // dominant channels; white/gray/black and opposite fixture colors fail.
      NSUInteger pixel[4]{};
      [bitmap getPixel:pixel atX:x y:y];
      if (pixel[red] > 200 && pixel[red + 1] < 120 && pixel[red + 2] > 200)
        ++magenta;
      if (pixel[red + 1] > 200 && pixel[red + 1] > pixel[red] + 60 &&
          pixel[red + 1] > pixel[red + 2] + 60)
        ++green;
    }
  }
  NSRect frame = [view frame];
  NSView* parent = [view superview];
  auto value = CefDictionaryValue::Create();
  value->SetBool("visible", ![view isHiddenOrHasHiddenAncestor] && [[view window] isVisible]);
  value->SetDouble("x", frame.origin.x);
  value->SetDouble("y", [parent isFlipped] ? frame.origin.y : NSHeight([parent bounds]) - frame.origin.y - frame.size.height);
  value->SetDouble("width", frame.size.width);
  value->SetDouble("height", frame.size.height);
  value->SetInt("magentaPixels", magenta);
  value->SetInt("greenPixels", green);
  value->SetInt("pixelWidth", static_cast<int>([bitmap pixelsWide]));
  value->SetInt("pixelHeight", static_cast<int>([bitmap pixelsHigh]));
  value->SetInt("bitsPerSample", static_cast<int>([bitmap bitsPerSample]));
  value->SetInt("samplesPerPixel", static_cast<int>(samples));
  value->SetBool("alphaFirst", alpha_first);
  for (const auto& sample : {std::pair("magentaSample", 4), std::pair("greenSample", 2)}) {
    NSUInteger pixel[4]{};
    auto channels = CefListValue::Create();
    if (rgb8) {
      [bitmap getPixel:pixel atX:[bitmap pixelsWide] / sample.second y:[bitmap pixelsHigh] / 3];
      for (NSInteger channel = 0; channel < samples; ++channel)
        channels->SetInt(channel, static_cast<int>(pixel[channel]));
    }
    value->SetList(sample.first, channels);
  }
  NSView* child = [[view subviews] firstObject];
  value->SetDouble("childWidth", child ? NSWidth([child frame]) : 0);
  value->SetDouble("childHeight", child ? NSHeight([child frame]) : 0);
  value->SetString("childClass", child ? [NSStringFromClass([child class]) UTF8String] : "");
  value->SetBool("parentLayer", [parent wantsLayer]);
  value->SetBool("viewAutoresizesSubviews", [view autoresizesSubviews]);
  value->SetInt("childAutoresizingMask", static_cast<int>([child autoresizingMask]));
  value->SetBool("cefApplication", [NSApp conformsToProtocol:@protocol(CefAppProtocol)]);
  value->SetBool("tauriEventLoop", TaoEventLoopInstalled());
  value->SetBool("parentAttached", [view window] && [parent window] == [view window]);
  value->SetBool("appActive", [NSApp isActive]);
  value->SetInt("activationPolicy", static_cast<int>([NSApp activationPolicy]));
  value->SetBool("windowKey", [[view window] isKeyWindow]);
  value->SetBool("windowVisible", [[view window] isVisible]);
  value->SetBool("windowCanBecomeKey", [[view window] canBecomeKeyWindow]);
  value->SetString("delegateClass", [[NSApp delegate] class] ?
    [NSStringFromClass([[NSApp delegate] class]) UTF8String] : "");
  auto wrapper = CefValue::Create(); wrapper->SetDictionary(value);
  *evidence = CefWriteJSON(wrapper, JSON_WRITER_DEFAULT).ToString();
  return true;
}
}
