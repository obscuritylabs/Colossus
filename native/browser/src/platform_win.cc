#include "host_internal.h"
#include <windows.h>
#include <algorithm>
#include <cstring>
#include <vector>

namespace colossus {
int32_t PlatformAcceptanceActivate(CefWindowHandle handle) {
  HWND root = GetAncestor(handle, GA_ROOT);
  if (!IsWindow(handle) || !root) return COLOSSUS_CEF_CLOSED;
  ShowWindow(root, SW_SHOW);
  return SetForegroundWindow(root) ? COLOSSUS_CEF_OK : COLOSSUS_CEF_UNAVAILABLE;
}

int32_t PlatformAcceptanceTerminate(CefWindowHandle handle) {
  HWND root = GetAncestor(handle, GA_ROOT);
  return root && PostMessageW(root, WM_CLOSE, 0, 0) ? COLOSSUS_CEF_OK : COLOSSUS_CEF_CLOSED;
}

int32_t PlatformAcceptanceInput(CefWindowHandle handle) {
  if (!IsWindowVisible(handle) || GetForegroundWindow() != GetAncestor(handle, GA_ROOT))
    return COLOSSUS_CEF_UNAVAILABLE;
  RECT area{};
  if (!GetWindowRect(handle, &area)) return COLOSSUS_CEF_CLOSED;
  const double scale = GetDpiForWindow(handle) / 96.0;
  // The native test fixture owns this fixed input's CSS rectangle. No arbitrary
  // browser command, page selector, JS expression or input string is accepted.
  const int x = area.left + static_cast<int>(80 * scale);
  const int y = area.top + static_cast<int>(115 * scale);
  const int left = GetSystemMetrics(SM_XVIRTUALSCREEN), top = GetSystemMetrics(SM_YVIRTUALSCREEN);
  const int width = GetSystemMetrics(SM_CXVIRTUALSCREEN), height = GetSystemMetrics(SM_CYVIRTUALSCREEN);
  if (width <= 1 || height <= 1) return COLOSSUS_CEF_UNAVAILABLE;
  INPUT mouse[3]{};
  for (auto& input : mouse) input.type = INPUT_MOUSE;
  mouse[0].mi.dx = ((x - left) * 65535) / (width - 1);
  mouse[0].mi.dy = ((y - top) * 65535) / (height - 1);
  mouse[0].mi.dwFlags = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
  mouse[1].mi.dwFlags = MOUSEEVENTF_LEFTDOWN;
  mouse[2].mi.dwFlags = MOUSEEVENTF_LEFTUP;
  if (SendInput(3, mouse, sizeof(INPUT)) != 3) return COLOSSUS_CEF_UNAVAILABLE;
  std::vector<INPUT> keyboard;
  for (const wchar_t character : std::wstring(L"colossus")) {
    INPUT down{}; down.type = INPUT_KEYBOARD;
    down.ki.wScan = character; down.ki.dwFlags = KEYEVENTF_UNICODE;
    keyboard.push_back(down);
    INPUT up = down; up.ki.dwFlags |= KEYEVENTF_KEYUP; keyboard.push_back(up);
  }
  return SendInput(static_cast<UINT>(keyboard.size()), keyboard.data(), sizeof(INPUT)) == keyboard.size()
      ? COLOSSUS_CEF_OK : COLOSSUS_CEF_UNAVAILABLE;
}

bool PlatformAcceptanceEvidence(CefWindowHandle handle, const void* result,
                                size_t size, std::string* evidence) {
  if (!CefCurrentlyOn(TID_UI) || !IsWindow(handle) || !evidence || !result ||
      size > COLOSSUS_CEF_MAX_PROTOCOL_BYTES) return false;
  auto parsed = CefParseJSON(result, size, JSON_PARSER_RFC);
  if (!parsed || parsed->GetType() != VTYPE_DICTIONARY) return false;
  auto png = CefBase64Decode(parsed->GetDictionary()->GetString("data"));
  uint8_t header[24]{};
  if (!png || png->GetSize() < sizeof(header) || png->GetSize() > COLOSSUS_CEF_MAX_PROTOCOL_BYTES)
    return false;
  png->GetData(header, sizeof(header), 0);
  static const uint8_t signature[8] = {137, 80, 78, 71, 13, 10, 26, 10};
  if (std::memcmp(header, signature, 8) || std::memcmp(header + 12, "IHDR", 4)) return false;
  const auto dimension = [&header](size_t offset) {
    return (uint32_t(header[offset]) << 24) | (uint32_t(header[offset + 1]) << 16) |
           (uint32_t(header[offset + 2]) << 8) | uint32_t(header[offset + 3]);
  };
  const uint32_t pixel_width = dimension(16), pixel_height = dimension(20);
  if (!pixel_width || !pixel_height || uint64_t(pixel_width) * pixel_height > 16 * 1024 * 1024)
    return false;
  HWND parent = GetParent(handle), root = GetAncestor(handle, GA_ROOT);
  RECT screen{}, parent_rect{};
  if (!parent || !root || !GetWindowRect(handle, &screen) || !GetWindowRect(parent, &parent_rect)) return false;
  POINT origin{screen.left, screen.top};
  if (!ScreenToClient(parent, &origin)) return false;
  const bool visible = IsWindowVisible(handle) && IsWindowVisible(root) && !IsIconic(root);
  int magenta = 0, green = 0;
  bool compositor_captured = false;
  // Capture the actual Windows compositor at this HWND's screen rectangle.
  // Sampling is bounded at 512x512 independently of device scale/monitor size.
  if (visible) {
    const int width = screen.right - screen.left, height = screen.bottom - screen.top;
    const int sampled_width = std::min(width, 512), sampled_height = std::min(height, 512);
    if (sampled_width <= 0 || sampled_height <= 0) return false;
    HDC desktop = GetDC(nullptr), capture = desktop ? CreateCompatibleDC(desktop) : nullptr;
    BITMAPINFO info{}; info.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
    info.bmiHeader.biWidth = sampled_width; info.bmiHeader.biHeight = -sampled_height;
    info.bmiHeader.biPlanes = 1; info.bmiHeader.biBitCount = 32; info.bmiHeader.biCompression = BI_RGB;
    void* pixels = nullptr;
    HBITMAP bitmap = desktop ? CreateDIBSection(desktop, &info, DIB_RGB_COLORS, &pixels, nullptr, 0) : nullptr;
    HGDIOBJ previous = capture && bitmap ? SelectObject(capture, bitmap) : nullptr;
    bool captured = previous && StretchBlt(capture, 0, 0, sampled_width, sampled_height,
        desktop, screen.left, screen.top, width, height, SRCCOPY | CAPTUREBLT);
    if (captured) {
      const auto* bytes = static_cast<const uint8_t*>(pixels);
      for (int index = 0; index < sampled_width * sampled_height; ++index) {
        const int blue = bytes[index * 4], g = bytes[index * 4 + 1], red = bytes[index * 4 + 2];
        if (red > 200 && g < 120 && blue > 200) ++magenta;
        if (g > 200 && g > red + 60 && g > blue + 60) ++green;
      }
    }
    if (previous) SelectObject(capture, previous);
    if (bitmap) DeleteObject(bitmap);
    if (capture) DeleteDC(capture);
    if (desktop) ReleaseDC(nullptr, desktop);
    if (!captured) return false;
    compositor_captured = true;
  }
  const double scale = GetDpiForWindow(parent) / 96.0;
  if (scale <= 0) return false;
  DWORD pid = 0;
  const DWORD thread = GetWindowThreadProcessId(root, &pid);
  auto value = CefDictionaryValue::Create();
  value->SetBool("visible", visible);
  value->SetDouble("x", origin.x / scale); value->SetDouble("y", origin.y / scale);
  value->SetDouble("width", (screen.right - screen.left) / scale);
  value->SetDouble("height", (screen.bottom - screen.top) / scale);
  value->SetInt("magentaPixels", magenta); value->SetInt("greenPixels", green);
  value->SetInt("pixelWidth", pixel_width); value->SetInt("pixelHeight", pixel_height);
  value->SetInt("bitsPerSample", 8); value->SetInt("samplesPerPixel", 4);
  value->SetBool("alphaFirst", false);
  value->SetList("magentaSample", CefListValue::Create()); value->SetList("greenSample", CefListValue::Create());
  value->SetDouble("childWidth", screen.right - screen.left);
  value->SetDouble("childHeight", screen.bottom - screen.top);
  wchar_t class_name[256]{}; GetClassNameW(handle, class_name, 256);
  value->SetString("childClass", CefString(class_name));
  value->SetBool("parentLayer", false); value->SetBool("viewAutoresizesSubviews", false);
  value->SetInt("childAutoresizingMask", 0);
  // These are AppKit-specific fields and carry no claimed Windows evidence.
  value->SetBool("cefApplication", false); value->SetBool("tauriEventLoop", false);
  value->SetBool("parentAttached", pid == GetCurrentProcessId() && IsChild(root, handle));
  value->SetBool("appActive", GetForegroundWindow() == root); value->SetInt("activationPolicy", 0);
  value->SetBool("windowKey", GetForegroundWindow() == root);
  value->SetBool("windowVisible", IsWindowVisible(root)); value->SetBool("windowCanBecomeKey", IsWindowEnabled(root));
  value->SetString("delegateClass", "");
  value->SetBool("windowsOwningThread", thread == GetCurrentThreadId());
  value->SetBool("osCompositorCapture", compositor_captured);
  auto wrapper = CefValue::Create(); wrapper->SetDictionary(value);
  *evidence = CefWriteJSON(wrapper, JSON_WRITER_DEFAULT).ToString();
  return true;
}
}
