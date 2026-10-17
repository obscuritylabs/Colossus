#include "colossus_browser_presenter.h"
#include <windows.h>
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
HWND Parent() {
  constexpr wchar_t name[] = L"ColossusPresenterLifecycleProbeV1";
  WNDCLASSW registration{}; registration.lpfnWndProc = DefWindowProcW;
  registration.hInstance = GetModuleHandleW(nullptr); registration.lpszClassName = name;
  if (!RegisterClassW(&registration) && GetLastError() != ERROR_CLASS_ALREADY_EXISTS) return nullptr;
  return CreateWindowExW(0, name, L"Colossus native presenter acceptance", WS_OVERLAPPEDWINDOW,
    CW_USEDEFAULT, CW_USEDEFAULT, 160, 160, nullptr, nullptr, registration.hInstance, nullptr);
}
bool Prepare(HWND parent, Owner& owner, const std::vector<uint8_t>& pixels) {
  owner.view = colossus_presenter_create(reinterpret_cast<uintptr_t>(parent), &owner, Input, Focus);
  return owner.view && colossus_presenter_lease(owner.view, 1, 1500) == COLOSSUS_CEF_OK &&
    colossus_presenter_bounds(owner.view, {0, 0, 64, 64}) == COLOSSUS_CEF_OK &&
    colossus_presenter_frame(owner.view, 64, 64, pixels.data(), pixels.size()) == COLOSSUS_CEF_OK &&
    colossus_presenter_visible(owner.view, 1) == COLOSSUS_CEF_OK;
}
}
int main() {
  std::vector<uint8_t> pixels(64 * 64 * 4, 255);
  HWND first = Parent(); Owner owner;
  if (!first || !Prepare(first, owner, pixels)) return 1;
  const auto retired = owner.view;
  if (!DestroyWindow(first) || colossus_presenter_destroy(retired) != COLOSSUS_CEF_OK ||
      colossus_presenter_frame(retired, 64, 64, pixels.data(), pixels.size()) == COLOSSUS_CEF_OK) return 1;
  HWND second = Parent(); Owner replacement;
  if (!second || !Prepare(second, replacement, pixels) || replacement.view == retired ||
      colossus_presenter_visible(retired, 1) == COLOSSUS_CEF_OK) return 1;
  // Use actual native focus dispatch. Its synchronous callback destroys the
  // presenter, exercising WM_NCDESTROY while the previous dispatch is on-stack.
  ShowWindow(second, SW_SHOW); SetForegroundWindow(second);
  SetFocus(second);
  HWND child = GetWindow(second, GW_CHILD);
  if (!child) return 1;
  replacement.destroy_on_focus = true;
  SetFocus(child);
  if (!replacement.destroyed || colossus_presenter_destroy(replacement.view) != COLOSSUS_CEF_OK ||
      !DestroyWindow(second)) return 1;
  std::cout << "native_presenter_parent_close=passed native_presenter_reentrant_close=passed "
    "native_presenter_identity_reuse=passed\n";
  return 0;
}
