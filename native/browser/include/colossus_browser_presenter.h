#ifndef COLOSSUS_BROWSER_PRESENTER_H
#define COLOSSUS_BROWSER_PRESENTER_H
#include "colossus_cef_presentation.h"
#ifdef __cplusplus
extern "C" {
#endif
/* Trusted native presenter owns monotonic opaque identities and borrows callbacks
 * until destruction acknowledgement; OS handle/pointer reuse cannot redirect them.
 * Neither renderer nor model supplies the parent, owner, callback or pixelskey.
 * Every API requires the create thread; presented BGRA bytes are copied.
 */
typedef void (*colossus_presenter_input_fn)(void* owner, const colossus_cef_input* input);
typedef void (*colossus_presenter_focus_fn)(void* owner, int32_t focused);
uintptr_t colossus_presenter_create(uintptr_t parent, void* owner,
  colossus_presenter_input_fn input, colossus_presenter_focus_fn focus);
int32_t colossus_presenter_bounds(uintptr_t view, colossus_cef_bounds bounds);
/* Monotonic native epoch; expired/hidden epochs cannot be renewed. Native OS
 * timer clears cached pixels/focus after1500ms independent of new frame arrival.
 */
int32_t colossus_presenter_lease(uintptr_t view, uint64_t epoch, uint32_t lease_ms);
int32_t colossus_presenter_frame(uintptr_t view, uint32_t width, uint32_t height,
  const uint8_t* bgra, size_t bytes);
int32_t colossus_presenter_visible(uintptr_t view, int32_t visible);
int32_t colossus_presenter_destroy(uintptr_t view);
#ifdef __cplusplus
}
#endif
#endif
