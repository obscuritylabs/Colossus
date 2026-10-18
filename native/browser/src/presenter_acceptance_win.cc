#include "colossus_presenter_acceptance_win.h"
#include <windows.h>
#include <windowsx.h>
#include <dwmapi.h>
#include <imm.h>
#include <cstring>
#include <initializer_list>
#include <cstdio>
#include <commctrl.h>
#include <atomic>
#include <memory>

namespace {
constexpr UINT_PTR kKeyboardFixtureSubclass = 0x434f4c4b;
struct KeyboardFixture {
  HWND parent = nullptr;
  HANDLE entered = nullptr, done = nullptr;
  std::atomic<bool> sender_started{false}, sent{false};
  bool outer = false, started = false, nested = false, queued = false, returned = false;
  ~KeyboardFixture() {
    if (entered) CloseHandle(entered);
    if (done) CloseHandle(done);
  }
};
// Native UI-thread ownership retains callback state if removal is unacknowledged.
// The source-only fixture refuses a second admission until cleanup succeeds.
std::shared_ptr<KeyboardFixture> keyboard_fixture;
LRESULT CALLBACK KeyboardFixtureProcedure(HWND window, UINT message, WPARAM wparam,
    LPARAM lparam, UINT_PTR, DWORD_PTR context) {
  auto& fixture = *reinterpret_cast<KeyboardFixture*>(context);
  if (message == WM_KEYUP && wparam == VK_RETURN && fixture.outer) fixture.nested = true;
  if (message != WM_KEYDOWN || wparam != VK_RETURN || fixture.started)
    return DefSubclassProc(window, message, wparam, lparam);
  fixture.started = true; fixture.outer = true;
  SetEvent(fixture.entered);
  const ULONGLONG until = GetTickCount64() + 500;
  // Do not pump here: the pending sent keyup must enter through Tao's own
  // PeekMessageW while its outer Return handler is still on the stack.
  while (GetTickCount64() < until) {
    if (fixture.sender_started.load(std::memory_order_acquire) &&
        (HIWORD(GetQueueStatus(QS_SENDMESSAGE)) & QS_SENDMESSAGE)) {
      fixture.queued = true; break;
    }
    Sleep(0);
  }
  const LRESULT result = DefSubclassProc(window, message, wparam, lparam);
  fixture.outer = false; fixture.returned = true;
  return result;
}
DWORD WINAPI SendNestedKey(void* context) {
  std::unique_ptr<std::shared_ptr<KeyboardFixture>> retained(
      static_cast<std::shared_ptr<KeyboardFixture>*>(context));
  const auto fixture = *retained;
  if (WaitForSingleObject(fixture->entered, 600) == WAIT_OBJECT_0 &&
      fixture->parent == GetForegroundWindow() && IsWindowVisible(fixture->parent)) {
    DWORD_PTR result = 0;
    fixture->sender_started.store(true, std::memory_order_release);
    // A timed-out send carries no pointers; delayed delivery cannot access the
    // helper's state after the UI thread has removed its subclass.
    const bool sent = SendMessageTimeoutW(fixture->parent, WM_KEYUP, VK_RETURN,
        LPARAM(0xc01c0001), SMTO_BLOCK | SMTO_ABORTIFHUNG | SMTO_ERRORONEXIT,
        1000, &result) != 0;
    fixture->sent.store(sent, std::memory_order_release);
  }
  SetEvent(fixture->done);
  return 0;
}
}

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
int32_t PresenterFixtureKeyboardReentrancy(uintptr_t raw) {
  const HWND parent = reinterpret_cast<HWND>(raw);
  DWORD process = 0;
  if (keyboard_fixture || !IsWindow(parent) ||
      GetWindowThreadProcessId(parent, &process) != GetCurrentThreadId() ||
      process != GetCurrentProcessId() || GetAncestor(parent, GA_ROOT) != parent ||
      parent != GetForegroundWindow() || !IsWindowVisible(parent))
    return COLOSSUS_CEF_DENIED;
  auto fixture = std::make_shared<KeyboardFixture>();
  fixture->parent = parent;
  fixture->entered = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  fixture->done = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  if (!fixture->entered || !fixture->done) return COLOSSUS_CEF_UNAVAILABLE;
  keyboard_fixture = fixture;
  if (!SetWindowSubclass(parent, KeyboardFixtureProcedure, kKeyboardFixtureSubclass,
      reinterpret_cast<DWORD_PTR>(fixture.get()))) {
    keyboard_fixture.reset(); return COLOSSUS_CEF_UNAVAILABLE;
  }
  auto* retained = new std::shared_ptr<KeyboardFixture>(fixture);
  const HANDLE sender = CreateThread(nullptr, 0, SendNestedKey, retained, 0, nullptr);
  if (!sender) {
    delete retained;
    if (RemoveWindowSubclass(parent, KeyboardFixtureProcedure, kKeyboardFixtureSubclass))
      keyboard_fixture.reset();
    return COLOSSUS_CEF_UNAVAILABLE;
  }
  // The unsafe old Tao version self-deadlocks here. The external acceptance
  // runner must enforce its process timeout; only its disposable app is targeted.
  SendMessageW(parent, WM_KEYDOWN, VK_RETURN, LPARAM(0x001c0001));
  const bool finished = WaitForSingleObject(fixture->done, 1600) == WAIT_OBJECT_0;
  CloseHandle(sender);
  const bool removed = RemoveWindowSubclass(parent, KeyboardFixtureProcedure,
      kKeyboardFixtureSubclass) != FALSE;
  if (removed) keyboard_fixture.reset();
  return finished && removed && fixture->queued && fixture->nested && fixture->returned &&
      fixture->sent.load(std::memory_order_acquire) ? COLOSSUS_CEF_OK : COLOSSUS_CEF_UNAVAILABLE;
}
}
