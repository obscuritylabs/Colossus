#include "colossus_browser_presenter.h"
#import <Cocoa/Cocoa.h>
#include <map>
#include <vector>
#include <algorithm>
#include <chrono>

@interface ColossusOwnedPixels : NSView <NSTextInputClient> {
 @public
  void* owner;
  colossus_presenter_input_fn input;
  colossus_presenter_focus_fn focus;
  std::vector<uint8_t> pixels;
  uint32_t pixelWidth, pixelHeight;
  NSString* marked;
  NSTrackingArea* tracking;
  NSTimer* leaseTimer;
  uint64_t epoch;
  uint32_t lastModifiers;
  BOOL interpretingKey;
  BOOL retired;
  std::chrono::steady_clock::time_point deadline;
}
- (BOOL)liveInput;
- (void)sendInput:(const colossus_cef_input*)value;
- (void)sendFocus:(BOOL)value;
@end
namespace {
uint32_t Modifiers(NSEvent* event) {
  uint32_t flags = 0;
  if (event.modifierFlags & NSEventModifierFlagShift) flags |= 2;
  if (event.modifierFlags & NSEventModifierFlagControl) flags |= 4;
  if (event.modifierFlags & NSEventModifierFlagOption) flags |= 8;
  if (event.modifierFlags & NSEventModifierFlagCommand) flags |= 128;
  return flags;
}
int32_t Key(NSEvent* event) {
  switch (event.keyCode) {
    case 36: case 76: return 13; case 48: return 9; case 49: return 32;
    case 51: return 8; case 53: return 27; case 117: return 46;
    case 123: return 37; case 124: return 39; case 125: return 40; case 126: return 38;
    case 115: return 36; case 119: return 35; case 116: return 33; case 121: return 34;
  }
  NSString* text = event.charactersIgnoringModifiers;
  if (text.length != 1) return 0;
  const unichar unit = [text characterAtIndex:0];
  if (unit >= 'a' && unit <= 'z') return unit - 'a' + 'A';
  if ((unit >= 'A' && unit <= 'Z') || (unit >= '0' && unit <= '9')) return unit;
  switch (unit) {
    case ';': return 186; case '=': return 187; case ',': return 188; case '-': return 189;
    case '.': return 190; case '/': return 191; case '`': return 192; case '[': return 219;
    case '\\': return 220; case ']': return 221; case '\'': return 222;
  }
  return 0;
}
}
@implementation ColossusOwnedPixels
- (BOOL)isFlipped { return YES; }
- (BOOL)acceptsFirstResponder { return YES; }
- (BOOL)liveInput {
  return !retired && epoch && std::chrono::steady_clock::now() < deadline && !self.hidden &&
    self.window.keyWindow && NSApp.active && self.window.firstResponder == self;
}
- (void)sendInput:(const colossus_cef_input*)value { if (!retired) input(owner, value); }
- (void)sendFocus:(BOOL)value { if (!retired) focus(owner, value ? 1 : 0); }
- (BOOL)becomeFirstResponder {
  if (retired || !epoch || std::chrono::steady_clock::now() >= deadline || self.hidden || !self.window.keyWindow || !NSApp.active) return NO;
  [self retain]; [self sendFocus:YES]; const BOOL active = !retired; [self release]; return active;
}
- (BOOL)resignFirstResponder {
  [self retain]; lastModifiers = 0; [self unmarkText]; [self sendFocus:NO]; [self release]; return YES;
}
- (void)updateTrackingAreas {
  if (tracking) { [self removeTrackingArea:tracking]; [tracking release]; }
  tracking = [[NSTrackingArea alloc] initWithRect:self.bounds options:NSTrackingMouseMoved |
    NSTrackingActiveInKeyWindow | NSTrackingInVisibleRect owner:self userInfo:nil];
  [self addTrackingArea:tracking]; [super updateTrackingAreas];
}
- (void)drawRect:(NSRect)rect {
  [[NSColor whiteColor] setFill]; NSRectFill(rect);
  if (pixels.empty() || !epoch || std::chrono::steady_clock::now() >= deadline) return;
  CGDataProviderRef provider = CGDataProviderCreateWithData(nullptr, pixels.data(), pixels.size(), nullptr);
  if (!provider) return;
  CGColorSpaceRef colors = CGColorSpaceCreateDeviceRGB();
  if (!colors) { CGDataProviderRelease(provider); return; }
  CGImageRef image = CGImageCreate(pixelWidth, pixelHeight, 8, 32, pixelWidth * 4, colors,
    kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little, provider, nullptr, false, kCGRenderingIntentDefault);
  if (image) {
    CGContextRef context = [[NSGraphicsContext currentContext] CGContext];
    CGContextSaveGState(context); CGContextTranslateCTM(context, 0, self.bounds.size.height);
    CGContextScaleCTM(context, 1, -1); CGContextDrawImage(context, self.bounds, image);
    CGContextRestoreGState(context); CGImageRelease(image);
  }
  CGColorSpaceRelease(colors); CGDataProviderRelease(provider);
}
- (void)sendMouse:(NSEvent*)event kind:(uint32_t)kind button:(int32_t)button {
  if (retired || !epoch || std::chrono::steady_clock::now() >= deadline || self.hidden || !self.window.keyWindow || !NSApp.active || !pixelWidth || !pixelHeight) return;
  NSPoint point = [self convertPoint:event.locationInWindow fromView:nil];
  if (point.x < 0 || point.y < 0 || point.x >= self.bounds.size.width || point.y >= self.bounds.size.height) return;
  colossus_cef_input value{}; value.version = 1; value.kind = kind; value.button = button;
  value.x = static_cast<int32_t>(point.x * pixelWidth / self.bounds.size.width);
  value.y = static_cast<int32_t>(point.y * pixelHeight / self.bounds.size.height);
  value.modifiers = Modifiers(event);
  if (kind == COLOSSUS_CEF_MOUSE_WHEEL) {
    value.wheel_x = static_cast<int32_t>(std::max(-4096.0, std::min(4096.0, event.scrollingDeltaX)));
    value.wheel_y = static_cast<int32_t>(std::max(-4096.0, std::min(4096.0, event.scrollingDeltaY)));
  }
  [self sendInput:&value];
}
- (void)mouseMoved:(NSEvent*)event { [self sendMouse:event kind:COLOSSUS_CEF_MOUSE_MOVE button:0]; }
- (void)mouseDragged:(NSEvent*)event { [self mouseMoved:event]; }
- (void)mouseDown:(NSEvent*)event { [self retain]; [self.window makeFirstResponder:self]; [self sendMouse:event kind:COLOSSUS_CEF_MOUSE_DOWN button:0]; [self release]; }
- (void)mouseUp:(NSEvent*)event { [self sendMouse:event kind:COLOSSUS_CEF_MOUSE_UP button:0]; }
- (void)rightMouseDown:(NSEvent*)event { [self retain]; [self.window makeFirstResponder:self]; [self sendMouse:event kind:COLOSSUS_CEF_MOUSE_DOWN button:2]; [self release]; }
- (void)rightMouseUp:(NSEvent*)event { [self sendMouse:event kind:COLOSSUS_CEF_MOUSE_UP button:2]; }
- (void)otherMouseDown:(NSEvent*)event { [self retain]; [self.window makeFirstResponder:self]; [self sendMouse:event kind:COLOSSUS_CEF_MOUSE_DOWN button:1]; [self release]; }
- (void)otherMouseUp:(NSEvent*)event { [self sendMouse:event kind:COLOSSUS_CEF_MOUSE_UP button:1]; }
- (void)scrollWheel:(NSEvent*)event { [self sendMouse:event kind:COLOSSUS_CEF_MOUSE_WHEEL button:0]; }
- (void)keyDown:(NSEvent*)event {
  if (![self liveInput]) return;
  [self retain];
  colossus_cef_input value{}; value.version = 1; value.kind = COLOSSUS_CEF_KEY_DOWN;
  value.key_code = Key(event); value.modifiers = Modifiers(event);
  if (value.key_code) [self sendInput:&value];
  if ([self liveInput] && !(value.modifiers & (4 | 128))) {
    interpretingKey = value.key_code != 0; [self interpretKeyEvents:@[event]]; interpretingKey = NO;
  }
  [self release];
}
- (void)keyUp:(NSEvent*)event {
  if (![self liveInput]) return;
  colossus_cef_input value{}; value.version = 1; value.kind = COLOSSUS_CEF_KEY_UP;
  value.key_code = Key(event); value.modifiers = Modifiers(event);
  if (value.key_code) [self sendInput:&value];
}
- (void)flagsChanged:(NSEvent*)event {
  if (![self liveInput]) { lastModifiers = 0; return; }
  [self retain];
  const uint32_t current = Modifiers(event), changed = current ^ lastModifiers;
  lastModifiers = current;
  const uint32_t flags[] = {2, 4, 8, 128}; const int32_t keys[] = {16, 17, 18, 91};
  for (size_t i = 0; i < 4 && [self liveInput]; ++i) if (changed & flags[i]) {
    colossus_cef_input value{}; value.version = 1; value.modifiers = current; value.key_code = keys[i];
    value.kind = current & flags[i] ? COLOSSUS_CEF_KEY_DOWN : COLOSSUS_CEF_KEY_UP; [self sendInput:&value];
  }
  [self release];
}
- (void)insertText:(id)string replacementRange:(NSRange)range {
  (void)range;
  NSString* text = [string isKindOfClass:[NSAttributedString class]] ? [string string] : string;
  if (![self liveInput] || ![text isKindOfClass:[NSString class]] || text.length == 0 || text.length > 4096) return;
  std::vector<uint16_t> units(text.length); [text getCharacters:units.data() range:NSMakeRange(0, text.length)];
  colossus_cef_input value{}; value.version = 1; value.kind = COLOSSUS_CEF_IME_COMMIT;
  [marked release]; marked = nil;
  value.text = units.data(); value.text_units = units.size(); [self sendInput:&value];
}
- (void)setMarkedText:(id)string selectedRange:(NSRange)selection replacementRange:(NSRange)range {
  (void)selection; (void)range;
  NSString* text = [string isKindOfClass:[NSAttributedString class]] ? [string string] : string;
  if (![self liveInput] || ![text isKindOfClass:[NSString class]] || text.length > 4096) return;
  [marked release]; marked = [text copy];
}
- (void)unmarkText {
  [marked release]; marked = nil;
  colossus_cef_input value{}; value.version = 1; value.kind = COLOSSUS_CEF_IME_CANCEL; [self sendInput:&value];
}
- (BOOL)hasMarkedText { return marked.length > 0; }
- (NSRange)markedRange { return marked ? NSMakeRange(0, marked.length) : NSMakeRange(NSNotFound, 0); }
- (NSRange)selectedRange { return NSMakeRange(NSNotFound, 0); }
- (NSArray*)validAttributesForMarkedText { return @[]; }
- (NSAttributedString*)attributedSubstringForProposedRange:(NSRange)range actualRange:(NSRangePointer)actual {
  (void)range; if (actual) *actual = NSMakeRange(NSNotFound, 0); return nil;
}
- (NSUInteger)characterIndexForPoint:(NSPoint)point { (void)point; return NSNotFound; }
- (NSRect)firstRectForCharacterRange:(NSRange)range actualRange:(NSRangePointer)actual {
  (void)range; if (actual) *actual = NSMakeRange(NSNotFound, 0);
  return [self.window convertRectToScreen:[self convertRect:self.bounds toView:nil]];
}
- (void)doCommandBySelector:(SEL)selector {
  if (![self liveInput] || interpretingKey) return;
  int32_t key = 0;
  if (selector == @selector(deleteBackward:)) key = 8;
  else if (selector == @selector(insertNewline:)) key = 13;
  else if (selector == @selector(insertTab:)) key = 9;
  else if (selector == @selector(moveLeft:)) key = 37;
  else if (selector == @selector(moveUp:)) key = 38;
  else if (selector == @selector(moveRight:)) key = 39;
  else if (selector == @selector(moveDown:)) key = 40;
  if (!key) return;
  colossus_cef_input value{}; value.version = 1; value.kind = COLOSSUS_CEF_KEY_DOWN; value.key_code = key;
  [self retain]; [self sendInput:&value];
  if ([self liveInput]) { value.kind = COLOSSUS_CEF_KEY_UP; [self sendInput:&value]; }
  [self release];
}
- (void)expireLease:(NSTimer*)timer {
  (void)timer;
  if (epoch && std::chrono::steady_clock::now() >= deadline && !self.hidden) {
    [self retain]; pixels.clear(); pixels.shrink_to_fit(); lastModifiers = 0;
    [self setHidden:YES]; [self unmarkText]; [self sendFocus:NO]; [self release];
  }
}
- (void)dealloc { [marked release]; if (tracking) [tracking release]; [leaseTimer invalidate]; [leaseTimer release]; [super dealloc]; }
@end
namespace {
std::map<uintptr_t, ColossusOwnedPixels*> views;
uintptr_t next_handle = 1;
ColossusOwnedPixels* Lookup(uintptr_t handle) {
  if (![NSThread isMainThread]) return nil;
  const auto found = views.find(handle); return found == views.end() ? nil : found->second;
}
}
extern "C" uintptr_t colossus_presenter_create(uintptr_t parent, void* owner,
    colossus_presenter_input_fn input, colossus_presenter_focus_fn focus) {
  if (![NSThread isMainThread] || !parent || !owner || !input || !focus || !next_handle) return 0;
  // Parent is supplied only by the native owning application's trusted adapter.
  NSView* native_parent = reinterpret_cast<NSView*>(parent);
  if (![native_parent isKindOfClass:[NSView class]] || !native_parent.window) return 0;
  auto* view = [[ColossusOwnedPixels alloc] initWithFrame:NSMakeRect(0, 0, 16, 16)];
  view->owner = owner; view->input = input; view->focus = focus;
  [view setHidden:YES]; [native_parent addSubview:view];
  view->leaseTimer = [[NSTimer timerWithTimeInterval:.05 target:view selector:@selector(expireLease:) userInfo:nil repeats:YES] retain];
  [[NSRunLoop mainRunLoop] addTimer:view->leaseTimer forMode:NSRunLoopCommonModes];
  const uintptr_t handle = next_handle++; views.emplace(handle, view); return handle;
}
extern "C" int32_t colossus_presenter_bounds(uintptr_t handle, colossus_cef_bounds rect) {
  auto* view = Lookup(handle);
  if (!view || rect.x < 0 || rect.y < 0 || rect.width < 1 || rect.height < 1 || rect.width > 16384 || rect.height > 16384)
    return COLOSSUS_CEF_INVALID;
  const auto y = view.superview.flipped ? rect.y : view.superview.bounds.size.height - rect.y - rect.height;
  view.frame = NSMakeRect(rect.x, y, rect.width, rect.height); return COLOSSUS_CEF_OK;
}
extern "C" int32_t colossus_presenter_frame(uintptr_t handle, uint32_t width, uint32_t height,
    const uint8_t* pixels, size_t bytes) {
  auto* view = Lookup(handle);
  if (!view || !view->epoch || std::chrono::steady_clock::now() >= view->deadline || !pixels || !width || !height || width > 4096 || height > 4096 ||
      uint64_t(width) * height * 4 != bytes || bytes > COLOSSUS_CEF_MAX_FRAME_BYTES) return COLOSSUS_CEF_INVALID;
  view->pixels.assign(pixels, pixels + bytes); view->pixelWidth = width; view->pixelHeight = height;
  [view setNeedsDisplay:YES]; return COLOSSUS_CEF_OK;
}
extern "C" int32_t colossus_presenter_visible(uintptr_t handle, int32_t visible) {
  if (![NSThread isMainThread]) return COLOSSUS_CEF_WRONG_THREAD;
  if (!Lookup(handle) && visible == 0 && handle && handle < next_handle) return COLOSSUS_CEF_OK;
  auto* view = Lookup(handle); if (!view || (visible != 0 && visible != 1)) return COLOSSUS_CEF_INVALID;
  if (visible && (!view->epoch || std::chrono::steady_clock::now() >= view->deadline)) return COLOSSUS_CEF_DENIED;
  [view retain];
  if (!visible) {
    view->pixels.clear(); view->pixels.shrink_to_fit(); view->deadline = {}; view->lastModifiers = 0;
    [view setHidden:YES]; [view unmarkText]; [view sendFocus:NO];
  } else [view setHidden:NO];
  [view release]; return COLOSSUS_CEF_OK;
}
extern "C" int32_t colossus_presenter_destroy(uintptr_t handle) {
  if (![NSThread isMainThread]) return COLOSSUS_CEF_WRONG_THREAD;
  auto* view = Lookup(handle);
  if (!view) return handle && handle < next_handle ? COLOSSUS_CEF_OK : COLOSSUS_CEF_CLOSED;
  views.erase(handle); view->retired = YES; view->deadline = {}; [view->leaseTimer invalidate];
  [view->marked release]; view->marked = nil; [view removeFromSuperview]; [view release]; return COLOSSUS_CEF_OK;
}
extern "C" int32_t colossus_presenter_lease(uintptr_t handle, uint64_t epoch, uint32_t ttl) {
  auto* view = Lookup(handle);
  if (!view || !epoch || !ttl || ttl > 1500 || epoch < view->epoch ||
      (epoch == view->epoch && std::chrono::steady_clock::now() >= view->deadline)) return COLOSSUS_CEF_DENIED;
  if (epoch != view->epoch) { [view->marked release]; view->marked = nil; view->lastModifiers = 0; }
  view->epoch = epoch; view->deadline = std::chrono::steady_clock::now() + std::chrono::milliseconds(ttl);
  return COLOSSUS_CEF_OK;
}
