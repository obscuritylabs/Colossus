#ifndef COLOSSUS_CEF_PRESENTATION_H
#define COLOSSUS_CEF_PRESENTATION_H
#include "colossus_cef.h"
#ifdef __cplusplus
extern "C" {
#endif
#define COLOSSUS_CEF_PRESENTATION_VERSION 1u
#define COLOSSUS_CEF_MAX_FRAME_BYTES (16u * 1024u * 1024u)
/* Native supervisor/presenter only. CEF UI thread, exact owned tab/generation.
 * Pixels are untrusted BGRA; copy/validate them before handing to an OS API.
 * This channel carries presentation, never runtime authorization or raw CDP.
 */
typedef struct colossus_cef_frame {
  uint32_t version, width, height, stride;
  uint64_t tab, generation, viewport_generation, document_generation, sequence;
  size_t bytes;
} colossus_cef_frame;
enum colossus_cef_input_kind {
  COLOSSUS_CEF_MOUSE_MOVE = 1, COLOSSUS_CEF_MOUSE_DOWN = 2,
  COLOSSUS_CEF_MOUSE_UP = 3, COLOSSUS_CEF_MOUSE_WHEEL = 4,
  COLOSSUS_CEF_KEY_DOWN = 5, COLOSSUS_CEF_KEY_UP = 6,
  COLOSSUS_CEF_CHARACTER = 7, COLOSSUS_CEF_IME_COMMIT = 8,
  COLOSSUS_CEF_IME_CANCEL = 9
};
typedef struct colossus_cef_input {
  uint32_t version, kind, modifiers;
  int32_t x, y, button, wheel_x, wheel_y, key_code;
  const uint16_t* text;
  size_t text_units;
} colossus_cef_input;
/* Positive monotonically increasing viewport_generation replaces all cached
 * pixels/focus. At most4096physical pixels peraxis/16MiB perframe; device_scale
 * [0.5,4]. lease_ms<=1500. Hide/focusloss cancelsIME and queued input/pixels.
 */
int32_t colossus_cef_presentation_document(colossus_cef_tab tab, uint64_t generation,
  uint64_t* document_generation);
int32_t colossus_cef_presentation_configure(colossus_cef_tab tab, uint64_t generation,
  uint64_t viewport_generation, uint64_t document_generation, uint32_t width, uint32_t height,
  double device_scale, uint32_t lease_ms);
int32_t colossus_cef_presentation_visible(colossus_cef_tab tab, uint64_t generation,
  uint64_t viewport_generation, uint64_t document_generation, int32_t visible, uint32_t lease_ms);
int32_t colossus_cef_presentation_focus(colossus_cef_tab tab, uint64_t generation,
  uint64_t viewport_generation, uint64_t document_generation, int32_t focused);
/* Latest frame only; buffer too small is invalid and does not consume frame.
 * Busy means no fresh frame. The successful call copies/consumes owned bytes.
 */
int32_t colossus_cef_presentation_frame(colossus_cef_tab tab, uint64_t generation,
  uint64_t viewport_generation, uint64_t document_generation, colossus_cef_frame* frame,
  uint8_t* bytes, size_t capacity);
int32_t colossus_cef_presentation_input(colossus_cef_tab tab, uint64_t generation,
  uint64_t viewport_generation, uint64_t document_generation, const colossus_cef_input* input);
#ifdef __cplusplus
}
#endif
#endif
