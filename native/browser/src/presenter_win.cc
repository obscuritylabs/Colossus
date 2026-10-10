#include "colossus_browser_presenter.h"
#include <windows.h>
#include <windowsx.h>
#include <imm.h>
#include <map>
#include <vector>
#include <atomic>
#include <chrono>

namespace {
struct View {
  HWND window = nullptr;
  DWORD thread = 0;
  void* owner = nullptr;
  colossus_presenter_input_fn input = nullptr;
  colossus_presenter_focus_fn focus = nullptr;
  uint32_t width = 0, height = 0;
  std::vector<uint8_t> pixels;
  uint64_t epoch = 0;
  uint16_t high_surrogate = 0;
  std::chrono::steady_clock::time_point deadline{};
};
std::map<uintptr_t, View*> views;
std::map<HWND, uintptr_t> windows;
uintptr_t next_handle = 1;
std::atomic<DWORD> owner_thread{0};
constexpr wchar_t kClass[] = L"ColossusOwnedBrowserPixelsV1";
View* Lookup(uintptr_t handle) {
  if (owner_thread.load() != GetCurrentThreadId()) return nullptr;
  const auto found = views.find(handle);
  return found != views.end() && found->second->thread == GetCurrentThreadId() ? found->second : nullptr;
}
uint32_t Modifiers() {
  uint32_t flags = 0;
  if (GetKeyState(VK_SHIFT) & 0x8000) flags |= 2;
  if (GetKeyState(VK_CONTROL) & 0x8000) flags |= 4;
  if (GetKeyState(VK_MENU) & 0x8000) flags |= 8;
  return flags;
}
bool Live(const View& view) { return view.epoch && std::chrono::steady_clock::now() < view.deadline; }
LRESULT CALLBACK Procedure(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
  const auto identity = windows.find(window);
  if (identity == windows.end()) return DefWindowProcW(window, message, wparam, lparam);
  const uintptr_t handle = identity->second;
  const auto found = views.find(handle);
  if (found == views.end()) return DefWindowProcW(window, message, wparam, lparam);
  auto& view = *found->second;
  if (message == WM_NCDESTROY) {
    // Destroying the parent destroys children too. Retire the opaque identity
    // before callbacks, so a recycled HWND cannot reach the previous owner.
    const auto owner = view.owner; const auto focus = view.focus;
    windows.erase(identity); views.erase(found); KillTimer(window, 1);
    delete &view; focus(owner, 0);
    return DefWindowProcW(window, message, wparam, lparam);
  }
  if (message == WM_TIMER && wparam == 1 && !Live(view) && IsWindowVisible(window)) {
    const auto owner = view.owner; const auto focus = view.focus;
    view.high_surrogate = 0; view.pixels.clear(); view.pixels.shrink_to_fit();
    focus(owner, 0);
    if (Lookup(handle)) ShowWindow(window, SW_HIDE);
    return 0;
  }
  if (message == WM_SETFOCUS || message == WM_KILLFOCUS) {
    if (message == WM_KILLFOCUS) view.high_surrogate = 0;
    view.focus(view.owner, message == WM_SETFOCUS && Live(view) ? 1 : 0);
    return 0;
  }
  if (message == WM_PAINT) {
    PAINTSTRUCT paint{}; HDC dc = BeginPaint(window, &paint);
    if (Live(view) && !view.pixels.empty()) {
      RECT rect{}; GetClientRect(window, &rect);
      BITMAPINFO info{}; info.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
      info.bmiHeader.biWidth = view.width; info.bmiHeader.biHeight = -static_cast<LONG>(view.height);
      info.bmiHeader.biPlanes = 1; info.bmiHeader.biBitCount = 32; info.bmiHeader.biCompression = BI_RGB;
      StretchDIBits(dc, 0, 0, rect.right, rect.bottom, 0, 0, view.width, view.height,
          view.pixels.data(), &info, DIB_RGB_COLORS, SRCCOPY);
    } else FillRect(dc, &paint.rcPaint, static_cast<HBRUSH>(GetStockObject(WHITE_BRUSH)));
    EndPaint(window, &paint); return 0;
  }
  if (!Live(view) || !IsWindowVisible(window) || GetAncestor(window, GA_ROOT) != GetForegroundWindow())
    return DefWindowProcW(window, message, wparam, lparam);
  colossus_cef_input input{}; input.version = COLOSSUS_CEF_PRESENTATION_VERSION; input.modifiers = Modifiers();
  if (message >= WM_MOUSEFIRST && message <= WM_MOUSELAST) {
    RECT rect{}; GetClientRect(window, &rect);
    POINT point{GET_X_LPARAM(lparam), GET_Y_LPARAM(lparam)};
    if (message == WM_MOUSEWHEEL || message == WM_MOUSEHWHEEL) ScreenToClient(window, &point);
    if (!view.width || !view.height || rect.right <= 0 || rect.bottom <= 0 ||
        point.x < 0 || point.y < 0 || point.x >= rect.right || point.y >= rect.bottom) return 0;
    input.x = static_cast<int32_t>(int64_t(point.x) * view.width / rect.right);
    input.y = static_cast<int32_t>(int64_t(point.y) * view.height / rect.bottom);
    switch (message) {
      case WM_MOUSEMOVE: input.kind = COLOSSUS_CEF_MOUSE_MOVE; break;
      case WM_LBUTTONDOWN: case WM_MBUTTONDOWN: case WM_RBUTTONDOWN:
        SetFocus(window); input.kind = COLOSSUS_CEF_MOUSE_DOWN;
        input.button = message == WM_LBUTTONDOWN ? 0 : message == WM_MBUTTONDOWN ? 1 : 2; break;
      case WM_LBUTTONUP: case WM_MBUTTONUP: case WM_RBUTTONUP:
        input.kind = COLOSSUS_CEF_MOUSE_UP;
        input.button = message == WM_LBUTTONUP ? 0 : message == WM_MBUTTONUP ? 1 : 2; break;
      case WM_MOUSEWHEEL: input.kind = COLOSSUS_CEF_MOUSE_WHEEL;
        input.wheel_y = GET_WHEEL_DELTA_WPARAM(wparam); break;
      case WM_MOUSEHWHEEL: input.kind = COLOSSUS_CEF_MOUSE_WHEEL;
        input.wheel_x = GET_WHEEL_DELTA_WPARAM(wparam); break;
      default: return DefWindowProcW(window, message, wparam, lparam);
    }
    auto* live = Lookup(handle);
    if (live && Live(*live)) live->input(live->owner, &input);
    return 0;
  }
  if (message == WM_KEYDOWN || message == WM_KEYUP || message == WM_CHAR) {
    if (wparam > 65535) return 0;
    if (message == WM_CHAR) {
      const uint16_t unit = static_cast<uint16_t>(wparam);
      if (unit >= 0xd800 && unit <= 0xdbff) { view.high_surrogate = unit; return 0; }
      const uint16_t high = view.high_surrogate; view.high_surrogate = 0;
      if (unit >= 0xdc00 && unit <= 0xdfff) {
        if (!high) return 0;
        const uint16_t pair[] = {high, unit};
        input.kind = COLOSSUS_CEF_IME_COMMIT; input.text = pair; input.text_units = 2;
        view.input(view.owner, &input); return 0;
      }
    }
    input.kind = message == WM_KEYDOWN ? COLOSSUS_CEF_KEY_DOWN :
      message == WM_KEYUP ? COLOSSUS_CEF_KEY_UP : COLOSSUS_CEF_CHARACTER;
    input.key_code = static_cast<int32_t>(wparam); view.input(view.owner, &input); return 0;
  }
  if (message == WM_IME_COMPOSITION && (lparam & GCS_RESULTSTR)) {
    HIMC context = ImmGetContext(window);
    if (!context) return 0;
    const LONG size = ImmGetCompositionStringW(context, GCS_RESULTSTR, nullptr, 0);
    if (size > 0 && size <= 8192 && size % 2 == 0) {
      std::vector<uint16_t> text(size / 2);
      if (ImmGetCompositionStringW(context, GCS_RESULTSTR, text.data(), size) == size) {
        input.kind = COLOSSUS_CEF_IME_COMMIT; input.text = text.data(); input.text_units = text.size();
        view.input(view.owner, &input);
      }
    }
    ImmReleaseContext(window, context); return 0;
  }
  if (message == WM_IME_ENDCOMPOSITION) {
    input.kind = COLOSSUS_CEF_IME_CANCEL; view.input(view.owner, &input); return 0;
  }
  return DefWindowProcW(window, message, wparam, lparam);
}
}
extern "C" uintptr_t colossus_presenter_create(uintptr_t parent, void* owner,
    colossus_presenter_input_fn input, colossus_presenter_focus_fn focus) {
  HWND native_parent = reinterpret_cast<HWND>(parent); DWORD pid = 0;
  if (!owner || !input || !focus || GetWindowThreadProcessId(native_parent, &pid) != GetCurrentThreadId() ||
      pid != GetCurrentProcessId()) return 0;
  DWORD expected = 0;
  if (!owner_thread.compare_exchange_strong(expected, GetCurrentThreadId()) && expected != GetCurrentThreadId()) return 0;
  if (!next_handle) return 0;
  WNDCLASSW registration{}; registration.lpfnWndProc = Procedure;
  registration.hInstance = GetModuleHandleW(nullptr); registration.lpszClassName = kClass;
  registration.hCursor = LoadCursorW(nullptr, IDC_ARROW);
  if (!RegisterClassW(&registration) && GetLastError() != ERROR_CLASS_ALREADY_EXISTS) return 0;
  HWND window = CreateWindowExW(0, kClass, L"", WS_CHILD | WS_TABSTOP, 0, 0, 16, 16,
      native_parent, nullptr, registration.hInstance, nullptr);
  if (!window) return 0;
  const uintptr_t handle = next_handle++;
  auto* view = new View{}; view->window = window; view->thread = GetCurrentThreadId();
  view->owner = owner; view->input = input; view->focus = focus;
  views.emplace(handle, view); windows.emplace(window, handle);
  if (!SetTimer(window, 1, 50, nullptr)) {
    DestroyWindow(window); return 0;
  }
  return handle;
}
extern "C" int32_t colossus_presenter_bounds(uintptr_t handle, colossus_cef_bounds rect) {
  auto* view = Lookup(handle);
  if (!view || rect.x < 0 || rect.y < 0 || rect.width < 1 || rect.height < 1 ||
      rect.width > 16384 || rect.height > 16384) return COLOSSUS_CEF_INVALID;
  return SetWindowPos(view->window, nullptr, rect.x, rect.y, rect.width, rect.height,
      SWP_NOACTIVATE | SWP_NOZORDER) ? COLOSSUS_CEF_OK : COLOSSUS_CEF_CLOSED;
}
extern "C" int32_t colossus_presenter_frame(uintptr_t handle, uint32_t width, uint32_t height,
    const uint8_t* pixels, size_t bytes) {
  auto* view = Lookup(handle);
  if (!view || !Live(*view) || !pixels || !width || !height || width > 4096 || height > 4096 ||
      uint64_t(width) * height * 4 != bytes || bytes > COLOSSUS_CEF_MAX_FRAME_BYTES) return COLOSSUS_CEF_INVALID;
  view->pixels.assign(pixels, pixels + bytes); view->width = width; view->height = height;
  InvalidateRect(view->window, nullptr, FALSE); return COLOSSUS_CEF_OK;
}
extern "C" int32_t colossus_presenter_visible(uintptr_t handle, int32_t visible) {
  if (owner_thread.load() != GetCurrentThreadId()) return COLOSSUS_CEF_WRONG_THREAD;
  auto* view = Lookup(handle);
  if (!view && visible == 0 && handle && handle < next_handle) return COLOSSUS_CEF_OK;
  if (!view || (visible != 0 && visible != 1)) return COLOSSUS_CEF_INVALID;
  if (visible && !Live(*view)) return COLOSSUS_CEF_DENIED;
  const HWND window = view->window;
  if (!visible) {
    view->pixels.clear(); view->pixels.shrink_to_fit(); view->high_surrogate = 0;
    view->deadline = {}; ShowWindow(window, SW_HIDE);
    if (Lookup(handle) && GetFocus() == window) SetFocus(GetParent(window));
    view = Lookup(handle);
    if (view) view->focus(view->owner, 0);
    return COLOSSUS_CEF_OK;
  }
  ShowWindow(window, SW_SHOWNA);
  return COLOSSUS_CEF_OK;
}
extern "C" int32_t colossus_presenter_destroy(uintptr_t handle) {
  auto* view = Lookup(handle);
  if (owner_thread.load() != GetCurrentThreadId()) return COLOSSUS_CEF_WRONG_THREAD;
  if (!view) return handle && handle < next_handle ? COLOSSUS_CEF_OK : COLOSSUS_CEF_CLOSED;
  // WM_NCDESTROY owns retirement and deletion for explicit and parent teardown.
  return DestroyWindow(view->window) ? COLOSSUS_CEF_OK : COLOSSUS_CEF_CLOSED;
}
extern "C" int32_t colossus_presenter_lease(uintptr_t handle, uint64_t epoch, uint32_t ttl) {
  auto* view = Lookup(handle);
  if (!view || !epoch || !ttl || ttl > 1500 || epoch < view->epoch ||
      (epoch == view->epoch && !Live(*view))) return COLOSSUS_CEF_DENIED;
  if (epoch != view->epoch) view->high_surrogate = 0;
  view->epoch = epoch; view->deadline = std::chrono::steady_clock::now() + std::chrono::milliseconds(ttl);
  return COLOSSUS_CEF_OK;
}
