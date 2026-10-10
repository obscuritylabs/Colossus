#include "colossus_browser_presenter.h"
#import <Cocoa/Cocoa.h>
#include <iostream>
#include <vector>

namespace {
struct Owner { uintptr_t view = 0; bool destroy_on_focus = false, destroyed = false; };
void Input(void*, const colossus_cef_input*) {}
void Focus(void* raw, int32_t focused) {
  auto& owner = *static_cast<Owner*>(raw);
  if (focused && owner.destroy_on_focus) {
    owner.destroy_on_focus = false;
    owner.destroyed = colossus_presenter_destroy(owner.view) == COLOSSUS_CEF_OK;
  }
}
NSWindow* Parent() {
  auto* window = [[NSWindow alloc] initWithContentRect:NSMakeRect(100, 100, 160, 160)
    styleMask:NSWindowStyleMaskTitled backing:NSBackingStoreBuffered defer:NO];
  [window setReleasedWhenClosed:NO]; return window;
}
bool Prepare(NSWindow* window, Owner& owner, const std::vector<uint8_t>& pixels) {
  owner.view = colossus_presenter_create(reinterpret_cast<uintptr_t>(window.contentView), &owner, Input, Focus);
  return owner.view && colossus_presenter_lease(owner.view, 1, 1500) == COLOSSUS_CEF_OK &&
    colossus_presenter_bounds(owner.view, {0, 0, 64, 64}) == COLOSSUS_CEF_OK &&
    colossus_presenter_frame(owner.view, 64, 64, pixels.data(), pixels.size()) == COLOSSUS_CEF_OK &&
    colossus_presenter_visible(owner.view, 1) == COLOSSUS_CEF_OK;
}
}
int main() {
  @autoreleasepool {
    [NSApplication sharedApplication]; [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];
    std::vector<uint8_t> pixels(64 * 64 * 4, 255);
    NSWindow* first = Parent(); Owner owner;
    if (!Prepare(first, owner, pixels)) return 1;
    const auto retired = owner.view;
    [first close];
    if (colossus_presenter_destroy(retired) != COLOSSUS_CEF_OK ||
        colossus_presenter_frame(retired, 64, 64, pixels.data(), pixels.size()) == COLOSSUS_CEF_OK) return 1;
    [first release];
    NSWindow* second = Parent(); Owner replacement;
    if (!Prepare(second, replacement, pixels) || replacement.view == retired ||
        colossus_presenter_visible(retired, 1) == COLOSSUS_CEF_OK) return 1;
    [second makeKeyAndOrderFront:nil]; [NSApp activateIgnoringOtherApps:YES];
    [[NSRunLoop mainRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:.1]];
    [second makeFirstResponder:nil];
    NSView* child = second.contentView.subviews.lastObject;
    if (!child) return 1;
    replacement.destroy_on_focus = true;
    [second makeFirstResponder:child];
    if (!replacement.destroyed || colossus_presenter_destroy(replacement.view) != COLOSSUS_CEF_OK) return 1;
    [second close]; [second release];
    std::cout << "native_presenter_parent_close=passed native_presenter_reentrant_close=passed "
      "native_presenter_identity_reuse=passed\n";
  }
  return 0;
}
