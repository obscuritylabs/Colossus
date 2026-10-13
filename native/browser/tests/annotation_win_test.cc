#include "annotation_win.h"
#include <iostream>
#include <limits>
#define CHECK(condition) do { if (!(condition)) { std::cerr << #condition << '\n'; return 1; } } while(false)
int main() {
  using colossus::AnnotationOverlay;
  const auto now=AnnotationOverlay::Clock::now();
  AnnotationOverlay overlay;
  colossus_cef_annotation mark{10,30,40,20,1,4,{L'T',L'e',L's',L't'}};
  CHECK(overlay.Set(&mark,1,1000,now));
  std::vector<uint8_t> pixels(160*160*4,255); const auto original=pixels;
  overlay.Paint(pixels,160,160,2,now);
  CHECK(pixels!=original);
  const size_t edge=(60*160+20)*4;
  CHECK(pixels[edge]==45 && pixels[edge+1]==35 && pixels[edge+2]==200 && pixels[edge+3]==255);
  CHECK(!overlay.Expire(now+std::chrono::milliseconds(999)));
  CHECK(overlay.Expire(now+std::chrono::milliseconds(1000)));
  pixels=original; overlay.Paint(pixels,160,160,2,now); CHECK(pixels==original);
  CHECK(!overlay.Set(&mark,17,1000,now));
  CHECK(!overlay.Set(&mark,1,300001,now));
  mark.label[0]=0xd800; CHECK(!overlay.Set(&mark,1,1000,now));
  mark.label[0]=L'T'; mark.left=std::numeric_limits<double>::infinity(); CHECK(!overlay.Set(&mark,1,1000,now));
  mark.left=10; CHECK(overlay.Set(&mark,1,1000,now));
  auto truncated=std::vector<uint8_t>(10,42); overlay.Paint(truncated,160,160,2,now); CHECK(truncated==std::vector<uint8_t>(10,42));
  CHECK(overlay.Clear()); pixels=original; overlay.Paint(pixels,160,160,2,now); CHECK(pixels==original);
  std::cout<<"PASS Windows annotation labels, palette, DPI, bounds, Unicode, expiry and clear\n";
}
