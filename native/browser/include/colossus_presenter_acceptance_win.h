#ifndef COLOSSUS_PRESENTER_ACCEPTANCE_WIN_H
#define COLOSSUS_PRESENTER_ACCEPTANCE_WIN_H
#include "colossus_browser_presenter.h"
#if defined(_WIN32)
#ifdef __cplusplus
extern "C" {
#endif
/* Closed source-fixture evidence, addressed only by an owned opaque presenter.
 * Pixels come from the foreground desktop DC, never the submitted frame cache.
 */
typedef struct colossus_presenter_screen {
  uint32_t width, height, dpi, foreground, visible, hit_test;
  uint8_t center_bgra[4];
} colossus_presenter_screen;
int32_t colossus_presenter_screen_probe(uintptr_t view, colossus_presenter_screen* screen);
int32_t colossus_presenter_screen_capture(uintptr_t view, uint32_t width, uint32_t height,
  uint8_t* bgra, size_t bytes);
/* Fixed synthetic field location and key/Unicode sequence; no general OS input API. */
int32_t colossus_presenter_fixture_input(uintptr_t view);
/* Fixed parent-window Return sequence with a reentrant sent keyup. This addresses
 * only the foreground root of this exact owned presenter, never another app.
 */
int32_t colossus_presenter_fixture_keyboard_reentrancy(uintptr_t view);
#ifdef __cplusplus
}
namespace colossus {
int32_t PresenterScreenProbe(uintptr_t window, colossus_presenter_screen* screen);
int32_t PresenterScreenCapture(uintptr_t window, uint32_t width, uint32_t height,
  uint8_t* bgra, size_t bytes);
int32_t PresenterFixtureInput(uintptr_t window);
int32_t PresenterFixtureKeyboardReentrancy(uintptr_t window);
}
#endif
#endif
#endif
