#ifndef COLOSSUS_AGENT_INDICATOR_WIN_H
#define COLOSSUS_AGENT_INDICATOR_WIN_H
#include <chrono>
#include <cstdint>
#include <vector>

namespace colossus {
// Presentation-only decoration. It never changes DOM, browser input or the OS cursor.
class AgentIndicator {
 public:
  using Clock = std::chrono::steady_clock;
  bool Set(uint32_t kind, double x, double y, double left, double top,
           double width, double height, Clock::time_point now, uint32_t ttl_ms = 2200);
  bool Clear();
  bool Expire(Clock::time_point now);
  void Paint(std::vector<uint8_t>& pixels, uint32_t width, uint32_t height,
             double scale, Clock::time_point now) const;
 private:
  uint32_t kind_ = 0;
  double x_ = 0, y_ = 0, left_ = 0, top_ = 0, width_ = 0, height_ = 0;
  Clock::time_point deadline_{};
};
}
#endif
