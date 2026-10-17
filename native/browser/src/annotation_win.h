#ifndef COLOSSUS_ANNOTATION_WIN_H
#define COLOSSUS_ANNOTATION_WIN_H
#include "colossus_cef_presentation.h"
#include <chrono>
#include <vector>
namespace colossus {
class AnnotationOverlay {
 public:
  using Clock = std::chrono::steady_clock;
  bool Set(const colossus_cef_annotation* marks, size_t count, uint32_t ttl_ms, Clock::time_point now);
  bool Clear();
  bool Expire(Clock::time_point now);
  void Paint(std::vector<uint8_t>& pixels, uint32_t width, uint32_t height, double scale, Clock::time_point now) const;
 private:
  std::vector<colossus_cef_annotation> marks_;
  Clock::time_point deadline_{};
};
}
#endif
