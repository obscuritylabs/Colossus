#include "host_internal.h"
#include "annotation_win.h"
#include "include/cef_image.h"
#include "include/cef_task.h"
#include <cmath>
#include <cstring>

extern "C" int32_t colossus_cef_annotation_png(const uint8_t* png, size_t bytes,
    const colossus_cef_annotation* marks, size_t count, double scale,
    uint8_t* output, size_t capacity, size_t* written) {
  if (!CefCurrentlyOn(TID_UI) || !png || bytes < 33 || bytes > 4 * 1024 * 1024 ||
      !output || !written || !capacity || capacity > 4 * 1024 * 1024 ||
      !std::isfinite(scale) || scale < .5 || scale > 4) return COLOSSUS_CEF_INVALID;
  *written = 0;
  const uint8_t signature[] = {137,80,78,71,13,10,26,10};
  if (std::memcmp(png,signature,8)) return COLOSSUS_CEF_INVALID;
  const auto dimension = [&](size_t offset) { return (uint32_t(png[offset])<<24) |
    (uint32_t(png[offset+1])<<16) | (uint32_t(png[offset+2])<<8) | png[offset+3]; };
  const auto w = dimension(16), h = dimension(20);
  if (!w || !h || w > 4096 || h > 4096 || uint64_t(w)*h*4 > 16*1024*1024) return COLOSSUS_CEF_INVALID;
  colossus::AnnotationOverlay overlay;
  const auto now = colossus::AnnotationOverlay::Clock::now();
  if (!count || !overlay.Set(marks,count,1000,now)) return COLOSSUS_CEF_INVALID;
  auto image = CefImage::CreateImage();
  if (!image->AddPNG(1, png, bytes)) return COLOSSUS_CEF_INVALID;
  int width=0, height=0;
  auto bitmap=image->GetAsBitmap(1,CEF_COLOR_TYPE_BGRA_8888,CEF_ALPHA_TYPE_PREMULTIPLIED,width,height);
  if (!bitmap || width!=static_cast<int>(w) || height!=static_cast<int>(h) || bitmap->GetSize()!=uint64_t(w)*h*4) return COLOSSUS_CEF_INVALID;
  std::vector<uint8_t> pixels(bitmap->GetSize());
  bitmap->GetData(pixels.data(),pixels.size(),0);
  overlay.Paint(pixels,w,h,scale,now);
  auto marked = CefImage::CreateImage();
  const bool added=marked->AddBitmap(1,width,height,CEF_COLOR_TYPE_BGRA_8888,CEF_ALPHA_TYPE_PREMULTIPLIED,pixels.data(),pixels.size());
  volatile uint8_t* private_pixels=pixels.data();
  for (size_t i=0;i<pixels.size();++i) private_pixels[i]=0;
  overlay.Clear();
  if (!added) return COLOSSUS_CEF_UNAVAILABLE;
  auto encoded=marked->GetAsPNG(1,true,width,height);
  if (!encoded || encoded->GetSize()>capacity) return COLOSSUS_CEF_INVALID;
  *written=encoded->GetData(output,capacity,0);
  return *written==encoded->GetSize() ? COLOSSUS_CEF_OK : COLOSSUS_CEF_INVALID;
}
