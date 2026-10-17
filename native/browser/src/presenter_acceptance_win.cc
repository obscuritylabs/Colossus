#include "colossus_presenter_acceptance_win.h"
#include <windows.h>
#include <windowsx.h>
#include <dwmapi.h>
#include <imm.h>
#include <cstring>
#include <initializer_list>
#include <cstdio>

namespace colossus {
int32_t PresenterScreenProbe(uintptr_t raw, colossus_presenter_screen* screen) {
  HWND window = reinterpret_cast<HWND>(raw);
  RECT rect{};
  if (!screen || !GetClientRect(window, &rect) || rect.right < 1 || rect.bottom < 1)
    return COLOSSUS_CEF_CLOSED;
  *screen = {};
  screen->width = rect.right; screen->height = rect.bottom;
  screen->dpi = GetDpiForWindow(window);
  screen->foreground = GetAncestor(window, GA_ROOT) == GetForegroundWindow();
  screen->visible = IsWindowVisible(window);
  POINT center{rect.right / 2, rect.bottom / 2};
  if (!ClientToScreen(window, &center)) return COLOSSUS_CEF_CLOSED;
  screen->hit_test = WindowFromPoint(center) == window;
  static bool reported_styles = false;
  if (!reported_styles) {
    reported_styles = true;
    std::fprintf(stderr, "native Windows compositor parent_extended_style=%llx view_extended_style=%llx\n",
        static_cast<unsigned long long>(GetWindowLongPtrW(GetAncestor(window, GA_ROOT), GWL_EXSTYLE)),
        static_cast<unsigned long long>(GetWindowLongPtrW(window, GWL_EXSTYLE)));
  }
  UpdateWindow(window); DwmFlush();
  HDC desktop = GetDC(nullptr);
  if (!desktop) return COLOSSUS_CEF_UNAVAILABLE;
  const COLORREF pixel = GetPixel(desktop, center.x, center.y);
  ReleaseDC(nullptr, desktop);
  if (pixel == CLR_INVALID) return COLOSSUS_CEF_UNAVAILABLE;
  screen->center_bgra[0] = GetBValue(pixel); screen->center_bgra[1] = GetGValue(pixel);
  screen->center_bgra[2] = GetRValue(pixel); screen->center_bgra[3] = 255;
  return COLOSSUS_CEF_OK;
}
int32_t PresenterScreenCapture(uintptr_t raw, uint32_t width, uint32_t height,
    uint8_t* bgra, size_t bytes) {
  HWND window = reinterpret_cast<HWND>(raw);
  RECT rect{}; POINT origin{};
  if (!bgra || !width || !height || width > 4096 || height > 4096 ||
      uint64_t(width) * height * 4 != bytes || bytes > COLOSSUS_CEF_MAX_FRAME_BYTES ||
      !GetClientRect(window, &rect) || uint32_t(rect.right) != width || uint32_t(rect.bottom) != height ||
      GetAncestor(window, GA_ROOT) != GetForegroundWindow() || !ClientToScreen(window, &origin))
    return COLOSSUS_CEF_DENIED;
  UpdateWindow(window); DwmFlush();
  HDC desktop = GetDC(nullptr), memory = desktop ? CreateCompatibleDC(desktop) : nullptr;
  BITMAPINFO info{}; info.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
  info.bmiHeader.biWidth = width; info.bmiHeader.biHeight = -static_cast<LONG>(height);
  info.bmiHeader.biPlanes = 1; info.bmiHeader.biBitCount = 32; info.bmiHeader.biCompression = BI_RGB;
  void* pixels = nullptr;
  HBITMAP bitmap = memory ? CreateDIBSection(desktop, &info, DIB_RGB_COLORS, &pixels, nullptr, 0) : nullptr;
  bool copied = false;
  if (bitmap) {
    HGDIOBJ previous = SelectObject(memory, bitmap);
    copied = BitBlt(memory, 0, 0, width, height, desktop, origin.x, origin.y, SRCCOPY | CAPTUREBLT);
    if (copied) {
      std::memcpy(bgra, pixels, bytes);
      for (size_t offset = 3; offset < bytes; offset += 4) bgra[offset] = 255;
    }
    SelectObject(memory, previous); DeleteObject(bitmap);
  }
  if (memory) DeleteDC(memory);
  if (desktop) ReleaseDC(nullptr, desktop);
  return copied ? COLOSSUS_CEF_OK : COLOSSUS_CEF_UNAVAILABLE;
}
int32_t PresenterFixtureInput(uintptr_t raw) {
  HWND window = reinterpret_cast<HWND>(raw);
  if (!IsWindowVisible(window) || GetAncestor(window, GA_ROOT) != GetForegroundWindow())
    return COLOSSUS_CEF_DENIED;
  const UINT dpi = GetDpiForWindow(window);
  const LPARAM point = MAKELPARAM(MulDiv(60, dpi, 96), MulDiv(100, dpi, 96));
  SendMessageW(window, WM_LBUTTONDOWN, MK_LBUTTON, point);
  SendMessageW(window, WM_LBUTTONUP, 0, point);
  SendMessageW(window, WM_KEYDOWN, 'A', 0);
  SendMessageW(window, WM_KEYUP, 'A', 0);
  for (const WPARAM character : {WPARAM('A'), WPARAM('b'), WPARAM('3'), WPARAM(0x3a9), WPARAM(0xd83d), WPARAM(0xde00)})
    SendMessageW(window, WM_CHAR, character, 0);
  SendMessageW(window, WM_IME_ENDCOMPOSITION, 0, 0);
  return COLOSSUS_CEF_OK;
}
}
