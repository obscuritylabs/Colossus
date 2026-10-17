#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include "annotation_win.h"
#include <algorithm>
#include <cmath>
#include <cstring>

namespace colossus {
bool AnnotationOverlay::Set(const colossus_cef_annotation* marks, size_t count,
                            uint32_t ttl_ms, Clock::time_point now) {
  if (!count) { Clear(); return true; }
  if (!marks || count > 16 || !ttl_ms || ttl_ms > 300000) return false;
  for (size_t i = 0; i < count; ++i) {
    const auto& mark = marks[i];
    if (mark.color > 3 || mark.label_units > 160 || mark.left < 0 || mark.top < 0 ||
        mark.width <= 0 || mark.height <= 0) return false;
    for (double value : {mark.left, mark.top, mark.width, mark.height})
      if (!std::isfinite(value) || value > 8192) return false;
    for (uint32_t j = 0; j < mark.label_units; ++j) {
      const uint16_t unit = mark.label[j];
      if (unit < 32 || (unit >= 127 && unit <= 159)) return false;
      if (unit >= 0xd800 && unit <= 0xdbff) {
        if (++j >= mark.label_units || mark.label[j] < 0xdc00 || mark.label[j] > 0xdfff) return false;
      } else if (unit >= 0xdc00 && unit <= 0xdfff) return false;
    }
  }
  marks_.assign(marks, marks + count);
  deadline_ = now + std::chrono::milliseconds(ttl_ms);
  return true;
}
bool AnnotationOverlay::Clear() {
  const bool changed = !marks_.empty();
  if (changed) SecureZeroMemory(marks_.data(), marks_.size() * sizeof(marks_[0]));
  marks_.clear(); deadline_ = {}; return changed;
}
bool AnnotationOverlay::Expire(Clock::time_point now) {
  return !marks_.empty() && now >= deadline_ ? Clear() : false;
}
void AnnotationOverlay::Paint(std::vector<uint8_t>& pixels, uint32_t width,
                             uint32_t height, double scale, Clock::time_point now) const {
  if (marks_.empty() || now >= deadline_ || !width || !height || width > 4096 || height > 4096 ||
      !std::isfinite(scale) || scale < .5 || scale > 4 ||
      pixels.size() != uint64_t(width) * height * 4 || pixels.size() > 16 * 1024 * 1024) return;
  BITMAPINFO info{}; info.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
  info.bmiHeader.biWidth = static_cast<LONG>(width); info.bmiHeader.biHeight = -static_cast<LONG>(height);
  info.bmiHeader.biPlanes = 1; info.bmiHeader.biBitCount = 32; info.bmiHeader.biCompression = BI_RGB;
  void* bits = nullptr;
  HDC dc = CreateCompatibleDC(nullptr);
  HBITMAP bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &bits, nullptr, 0);
  if (!dc || !bitmap || !bits) { if (bitmap) DeleteObject(bitmap); if (dc) DeleteDC(dc); return; }
  auto old_bitmap = SelectObject(dc, bitmap);
  std::memcpy(bits, pixels.data(), pixels.size());
  HFONT font = CreateFontW(-static_cast<int>(std::ceil(14 * scale)), 0, 0, 0, FW_SEMIBOLD,
    FALSE, FALSE, FALSE, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
    ANTIALIASED_QUALITY, DEFAULT_PITCH, L"Segoe UI");
  auto old_font = font ? SelectObject(dc, font) : nullptr;
  constexpr COLORREF colors[] = {RGB(30,100,220),RGB(200,35,45),RGB(0,125,65),RGB(190,140,0)};
  for (const auto& mark : marks_) {
    RECT rect{static_cast<LONG>(std::floor(mark.left * scale)), static_cast<LONG>(std::floor(mark.top * scale)),
      static_cast<LONG>(std::ceil((mark.left + mark.width) * scale)), static_cast<LONG>(std::ceil((mark.top + mark.height) * scale))};
    HBRUSH brush = CreateSolidBrush(colors[mark.color]);
    if (!brush) continue;
    for (int stroke = 0; stroke < std::max(2, static_cast<int>(std::ceil(3 * scale))); ++stroke) {
      FrameRect(dc, &rect, brush); InflateRect(&rect, -1, -1);
    }
    if (mark.label_units) {
      RECT label{std::clamp(static_cast<LONG>(mark.left * scale), 0L, static_cast<LONG>(width - 1)),
        std::clamp(static_cast<LONG>(mark.top * scale) - static_cast<LONG>(std::ceil(24 * scale)), 0L, static_cast<LONG>(height - 1)),
        static_cast<LONG>(width), static_cast<LONG>(height)};
      RECT measured{0,0,0,0};
      const auto text = reinterpret_cast<const wchar_t*>(mark.label);
      DrawTextW(dc, text, static_cast<int>(mark.label_units), &measured, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
      label.right = std::min(static_cast<LONG>(width), label.left + measured.right + static_cast<LONG>(8 * scale));
      label.bottom = std::min(static_cast<LONG>(height), label.top + measured.bottom + static_cast<LONG>(4 * scale));
      FillRect(dc, &label, brush); SetBkMode(dc, TRANSPARENT); SetTextColor(dc, RGB(255,255,255));
      label.left += static_cast<LONG>(4 * scale); label.top += static_cast<LONG>(2 * scale);
      DrawTextW(dc, text, static_cast<int>(mark.label_units), &label, DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS);
    }
    DeleteObject(brush);
  }
  GdiFlush();
  const auto source = static_cast<const uint8_t*>(bits);
  for (size_t i = 0; i < pixels.size(); i += 4) {
    if (std::memcmp(pixels.data() + i, source + i, 3)) {
      std::memcpy(pixels.data() + i, source + i, 3); pixels[i + 3] = 255;
    }
  }
  SecureZeroMemory(bits, pixels.size());
  if (old_font) SelectObject(dc, old_font);
  if (font) DeleteObject(font);
  SelectObject(dc, old_bitmap); DeleteObject(bitmap); DeleteDC(dc);
}
}
