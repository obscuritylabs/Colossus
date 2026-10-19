#include "agent_indicator_win.h"
#include <cmath>
#include <iostream>
#include <limits>

#define CHECK(condition) do { if (!(condition)) { std::cerr << #condition << '\n'; return 1; } } while (false)
int main() {
  using colossus::AgentIndicator;
  const auto now = AgentIndicator::Clock::now();
  AgentIndicator indicator;
  CHECK(indicator.Set(1, 20, 20, 10, 10, 20, 20, now));
  std::vector<uint8_t> pixels(80 * 80 * 4, 127);
  const auto original = pixels;
  indicator.Paint(pixels, 80, 80, 2, now);
  CHECK(pixels != original);
  const size_t edge = (size_t(25) * 80 + 19) * 4;
  CHECK(pixels[edge] == 0 && pixels[edge + 3] == 255);
  CHECK(!indicator.Expire(now + std::chrono::milliseconds(2200)));
  pixels = original;
  indicator.Paint(pixels, 80, 80, 2, now + std::chrono::seconds(3));
  CHECK(pixels != original);
  CHECK(!indicator.Expire(now + std::chrono::milliseconds(7999)));
  CHECK(indicator.Expire(now + std::chrono::milliseconds(8000)));
  pixels = original;
  indicator.Paint(pixels, 80, 80, 2, now + std::chrono::seconds(8));
  CHECK(pixels == original);
  CHECK(!indicator.Set(1, 20, 20, 0, 0, 20, 20, now, 8001));
  CHECK(indicator.Set(1, 20, 20, 0, 0, 20, 20, now, 500));
  CHECK(indicator.Expire(now + std::chrono::milliseconds(500)));
  CHECK(!indicator.Set(2, std::numeric_limits<double>::infinity(), 20, 0, 0, 20, 20, now));
  CHECK(!indicator.Set(3, 20, 20, 0, 0, 20, 20, now));
  CHECK(indicator.Set(2, 1, 1, -10, -10, 20, 20, now));
  indicator.Paint(pixels, 80, 80, 4, now);
  CHECK(pixels.size() == original.size());
  CHECK(indicator.Clear());
  pixels = original;
  indicator.Paint(pixels, 80, 80, 4, now);
  CHECK(pixels == original);
  std::cout << "PASS Windows agent indicator geometry, DPI, clipping, expiry and clear\n";
}
