#include "agent_indicator_win.h"
#include <algorithm>
#include <cmath>

namespace colossus {
bool AgentIndicator::Set(uint32_t kind, double x, double y, double left, double top,
                         double width, double height, Clock::time_point now, uint32_t ttl_ms) {
  if (kind == 0) { Clear(); return true; }
  if (kind > 2 || width <= 0 || height <= 0 || !ttl_ms || ttl_ms > 2200) return false;
  for (double value : {x, y, left, top, width, height})
    if (!std::isfinite(value) || std::abs(value) > 1000000) return false;
  kind_ = kind; x_ = x; y_ = y; left_ = left; top_ = top;
  width_ = width; height_ = height;
  deadline_ = now + std::chrono::milliseconds(ttl_ms);
  return true;
}
bool AgentIndicator::Clear() {
  const bool changed = kind_ != 0;
  kind_ = 0; deadline_ = {};
  return changed;
}
bool AgentIndicator::Expire(Clock::time_point now) {
  return kind_ && now >= deadline_ ? Clear() : false;
}
void AgentIndicator::Paint(std::vector<uint8_t>& pixels, uint32_t width,
                           uint32_t height, double scale, Clock::time_point now) const {
  if (!kind_ || now >= deadline_ || !width || !height ||
      !std::isfinite(scale) || scale < .5 || scale > 4 ||
      pixels.size() != uint64_t(width) * height * 4) return;
  // Black/white double strokes remain distinguishable over arbitrary page colors.
  // Geometry is DIP; only the authenticated native frame determines physical scale.
  const auto put = [&](int x, int y, uint8_t shade) {
    if (x < 0 || y < 0 || x >= int(width) || y >= int(height)) return;
    const size_t index = (size_t(y) * width + size_t(x)) * 4;
    pixels[index] = pixels[index + 1] = pixels[index + 2] = shade;
    pixels[index + 3] = 255;
  };
  const int stroke = std::max(1, int(std::ceil(scale)));
  const int left = int(std::floor(left_ * scale)), top = int(std::floor(top_ * scale));
  const int right = int(std::ceil((left_ + width_) * scale));
  const int bottom = int(std::ceil((top_ + height_) * scale));
  for (int y = std::max(0, top - 2 * stroke); y < std::min(int(height), bottom + 2 * stroke); ++y) {
    for (int x = std::max(0, left - 2 * stroke); x < std::min(int(width), right + 2 * stroke); ++x) {
      const int edge = std::min({x - left, right - 1 - x, y - top, bottom - 1 - y});
      if (edge < 0) put(x, y, 0);
      else if (edge < stroke) put(x, y, 255);
    }
  }
  const int anchor_x = int(std::round(x_ * scale)), anchor_y = int(std::round(y_ * scale));
  if (kind_ == 1) {
    const int radius = int(std::ceil(14 * scale));
    for (int y = -radius - stroke; y <= radius + stroke; ++y) {
      for (int x = -radius - stroke; x <= radius + stroke; ++x) {
        const double distance = std::hypot(x, y);
        if (std::abs(distance - radius) <= stroke)
          put(anchor_x + x, anchor_y + y, distance < radius ? 255 : 0);
      }
    }
  }
  // A separate outlined arrow identifies the agent's target without moving the human mouse.
  const int extent = int(std::ceil(24 * scale));
  for (int y = 0; y < extent; ++y) {
    for (int x = 0; x < extent; ++x) {
      const double dip_x = x / scale, dip_y = y / scale;
      const bool arrow = dip_y <= 19 && dip_x <= dip_y * .65 &&
          (dip_y <= 12 || dip_x >= (dip_y - 12) * .7);
      if (arrow) put(anchor_x + x, anchor_y + y,
          dip_x < 1.5 || dip_y < 2 || dip_x > dip_y * .65 - 1.5 || dip_y > 17 ? 0 : 255);
    }
  }
}
}
