#include "host_internal.h"
#include "colossus_cef_presentation.h"
#include "include/cef_render_handler.h"
#include "include/cef_task.h"
#include <algorithm>
#include <cmath>
#include <cstring>
#include <limits>

namespace colossus {
namespace {
struct Presentation {
  uint64_t generation = 0, viewport = 0, document = 0, sequence = 0;
  uint32_t width = 0, height = 0, pixel_width = 0, pixel_height = 0;
  double scale = 1;
  bool visible = false, focused = false;
  std::chrono::steady_clock::time_point deadline{};
  std::vector<uint8_t> frame;
};
std::map<colossus_cef_tab, Presentation> presentations;
std::map<colossus_cef_tab, std::pair<uint64_t, uint64_t>> documents;
size_t retained = 0;
constexpr size_t kRetainedLimit = 2 * COLOSSUS_CEF_MAX_FRAME_BYTES;
void Clear(Presentation& state) {
  volatile uint8_t* bytes = state.frame.data();
  for (size_t index = 0; index < state.frame.size(); ++index) bytes[index] = 0;
  retained -= state.frame.size();
  state.frame.clear();
  state.frame.shrink_to_fit();
}
int32_t Lookup(colossus_cef_tab tab, uint64_t generation, uint64_t viewport, uint64_t document,
               Tab** browser, Presentation** state) {
  const int32_t status = Resolve(tab, generation, browser);
  if (status) return status;
  if (!headless) return COLOSSUS_CEF_UNAVAILABLE;
  const auto found = presentations.find(tab);
  if (found == presentations.end() || found->second.generation != generation ||
      found->second.viewport != viewport || found->second.document != document || !viewport || !document) return COLOSSUS_CEF_DENIED;
  *state = &found->second;
  return COLOSSUS_CEF_OK;
}
bool Lease(const Presentation& state) {
  return state.visible && std::chrono::steady_clock::now() < state.deadline;
}
bool Utf16(const uint16_t* text, size_t count) {
  if (!text || !count || count > 4096) return false;
  for (size_t index = 0; index < count; ++index) {
    const uint16_t unit = text[index];
    if (!unit) return false;
    if (unit >= 0xd800 && unit <= 0xdbff) {
      if (++index >= count || text[index] < 0xdc00 || text[index] > 0xdfff) return false;
    } else if (unit >= 0xdc00 && unit <= 0xdfff) return false;
  }
  return true;
}
}

bool PresentationRect(colossus_cef_tab tab, uint64_t generation, CefRect* rect,
                      double* scale) {
  const auto found = presentations.find(tab);
  if (found == presentations.end() || found->second.generation != generation) return false;
  *rect = CefRect(0, 0, found->second.width, found->second.height);
  *scale = found->second.scale;
  return true;
}
bool PresentationFocused(colossus_cef_tab tab, uint64_t generation) {
  const auto found = presentations.find(tab);
  return found != presentations.end() && found->second.generation == generation &&
      found->second.focused && Lease(found->second);
}
void PresentationPaint(colossus_cef_tab tab, uint64_t generation,
                       CefRenderHandler::PaintElementType kind,
                       const void* pixels, int width, int height) {
  if (!CefCurrentlyOn(TID_UI) || kind != PET_VIEW || !pixels || width <= 0 || height <= 0)
    return;
  const auto found = presentations.find(tab);
  if (found == presentations.end() || found->second.generation != generation ||
      !Lease(found->second)) return;
  auto& state = found->second;
  if (width != static_cast<int>(std::ceil(state.width * state.scale)) ||
      height != static_cast<int>(std::ceil(state.height * state.scale))) return;
  const uint64_t size = uint64_t(width) * uint64_t(height) * 4;
  if (size > COLOSSUS_CEF_MAX_FRAME_BYTES || state.sequence == std::numeric_limits<uint64_t>::max()) return;
  Clear(state);
  for (auto& [other_tab, other] : presentations) {
    if (retained + size <= kRetainedLimit) break;
    if (other_tab != tab) Clear(other);
  }
  state.frame.assign(static_cast<const uint8_t*>(pixels),
                     static_cast<const uint8_t*>(pixels) + size);
  retained += state.frame.size();
  state.pixel_width = width; state.pixel_height = height;
  ++state.sequence;
}
void PresentationExpire() {
  for (auto& [tab, state] : presentations) {
    const auto browser = tabs.find(tab);
    if (browser == tabs.end() || browser->second.generation != state.generation ||
        browser->second.closing || (state.visible && !Lease(state))) {
      if (browser != tabs.end() && browser->second.browser) {
        auto host = browser->second.browser->GetHost();
        host->SetFocus(false); host->ImeCancelComposition(); host->WasHidden(true);
      }
      state.visible = false; state.focused = false; Clear(state);
    }
  }
}
void PresentationDocument(colossus_cef_tab tab, uint64_t generation) {
  auto& document = documents[tab];
  if (document.first != generation) document = {generation, 0};
  if (document.second != std::numeric_limits<uint64_t>::max()) ++document.second;
  const auto found = presentations.find(tab);
  if (found != presentations.end()) {
    auto& state = found->second;
    state.visible = false; state.focused = false; Clear(state);
    const auto browser = tabs.find(tab);
    if (browser != tabs.end() && browser->second.browser) {
      auto host = browser->second.browser->GetHost();
      host->SetFocus(false); host->ImeCancelComposition(); host->WasHidden(true);
    }
  }
}
void PresentationClosed(colossus_cef_tab tab) {
  const auto found = presentations.find(tab);
  if (found != presentations.end()) { Clear(found->second); presentations.erase(found); }
  documents.erase(tab);
}
}

extern "C" int32_t colossus_cef_presentation_document(colossus_cef_tab tab,
    uint64_t generation, uint64_t* document) {
  using namespace colossus;
  Tab* browser = nullptr;
  const auto status = Resolve(tab, generation, &browser);
  if (status) return status;
  if (!document || !headless) return COLOSSUS_CEF_INVALID;
  const auto found = documents.find(tab);
  if (found == documents.end() || found->second.first != generation ||
      !found->second.second || found->second.second == std::numeric_limits<uint64_t>::max()) return COLOSSUS_CEF_DENIED;
  *document = found->second.second; return COLOSSUS_CEF_OK;
}
extern "C" int32_t colossus_cef_presentation_configure(colossus_cef_tab tab,
    uint64_t generation, uint64_t viewport, uint64_t document, uint32_t width, uint32_t height,
    double scale, uint32_t lease_ms) {
  using namespace colossus;
  Tab* browser = nullptr;
  const auto status = Resolve(tab, generation, &browser);
  if (status) return status;
  if (!headless) return COLOSSUS_CEF_UNAVAILABLE;
  const auto native_document = documents.find(tab);
  if (native_document == documents.end() || native_document->second.first != generation ||
      native_document->second.second != document || !document) return COLOSSUS_CEF_DENIED;
  if (!viewport || !width || !height || width > 4096 || height > 4096 ||
      !std::isfinite(scale) || scale < .5 || scale > 4 || !lease_ms || lease_ms > 1500)
    return COLOSSUS_CEF_INVALID;
  const double pixel_width = std::ceil(width * scale), pixel_height = std::ceil(height * scale);
  if (pixel_width > 4096 || pixel_height > 4096 ||
      pixel_width * pixel_height * 4 > COLOSSUS_CEF_MAX_FRAME_BYTES) return COLOSSUS_CEF_INVALID;
  auto& state = presentations[tab];
  if (state.generation == generation && viewport <= state.viewport) return COLOSSUS_CEF_DENIED;
  Clear(state);
  state.generation = generation; state.viewport = viewport; state.document = document;
  state.width = width; state.height = height; state.scale = scale;
  state.visible = true; state.focused = false;
  state.deadline = std::chrono::steady_clock::now() + std::chrono::milliseconds(lease_ms);
  auto host = browser->browser->GetHost();
  host->SetFocus(false); host->ImeCancelComposition(); host->WasHidden(false);
  host->NotifyScreenInfoChanged(); host->WasResized();
  return COLOSSUS_CEF_OK;
}
extern "C" int32_t colossus_cef_presentation_visible(colossus_cef_tab tab,
    uint64_t generation, uint64_t viewport, uint64_t document, int32_t visible, uint32_t lease_ms) {
  using namespace colossus;
  Tab* browser = nullptr; Presentation* state = nullptr;
  const auto status = Lookup(tab, generation, viewport, document, &browser, &state);
  if (status) return status;
  if ((visible != 0 && visible != 1) || (visible && (!lease_ms || lease_ms > 1500)))
    return COLOSSUS_CEF_INVALID;
  if (visible && !Lease(*state)) {
    state->visible = false; state->focused = false; Clear(*state);
    browser->browser->GetHost()->SetFocus(false);
    browser->browser->GetHost()->ImeCancelComposition();
    browser->browser->GetHost()->WasHidden(true);
    return COLOSSUS_CEF_DENIED;
  }
  state->visible = visible != 0;
  state->deadline = std::chrono::steady_clock::now() + std::chrono::milliseconds(lease_ms);
  auto host = browser->browser->GetHost();
  if (!visible) { state->focused = false; host->SetFocus(false); host->ImeCancelComposition(); Clear(*state); }
  host->WasHidden(!visible);
  return COLOSSUS_CEF_OK;
}
extern "C" int32_t colossus_cef_presentation_focus(colossus_cef_tab tab,
    uint64_t generation, uint64_t viewport, uint64_t document, int32_t focused) {
  using namespace colossus;
  Tab* browser = nullptr; Presentation* state = nullptr;
  const auto status = Lookup(tab, generation, viewport, document, &browser, &state);
  if (status) return status;
  if ((focused != 0 && focused != 1) || (focused && !Lease(*state))) return COLOSSUS_CEF_DENIED;
  state->focused = focused != 0;
  browser->browser->GetHost()->SetFocus(focused != 0);
  if (!focused) { browser->browser->GetHost()->ImeCancelComposition(); Clear(*state); }
  return COLOSSUS_CEF_OK;
}
extern "C" int32_t colossus_cef_presentation_frame(colossus_cef_tab tab,
    uint64_t generation, uint64_t viewport, uint64_t document, colossus_cef_frame* frame,
    uint8_t* bytes, size_t capacity) {
  using namespace colossus;
  Tab* browser = nullptr; Presentation* state = nullptr;
  const auto status = Lookup(tab, generation, viewport, document, &browser, &state);
  if (status) return status;
  if (!Lease(*state)) return COLOSSUS_CEF_DENIED;
  if (state->frame.empty()) return COLOSSUS_CEF_BUSY;
  if (!frame || !bytes || capacity < state->frame.size() || capacity > COLOSSUS_CEF_MAX_FRAME_BYTES)
    return COLOSSUS_CEF_INVALID;
  *frame = {COLOSSUS_CEF_PRESENTATION_VERSION, state->pixel_width, state->pixel_height,
    state->pixel_width * 4, tab, generation, viewport, document, state->sequence, state->frame.size()};
  std::memcpy(bytes, state->frame.data(), state->frame.size()); Clear(*state);
  return COLOSSUS_CEF_OK;
}
extern "C" int32_t colossus_cef_presentation_input(colossus_cef_tab tab,
    uint64_t generation, uint64_t viewport, uint64_t document, const colossus_cef_input* input) {
  using namespace colossus;
  Tab* browser = nullptr; Presentation* state = nullptr;
  const auto status = Lookup(tab, generation, viewport, document, &browser, &state);
  if (status) return status;
  if (!Lease(*state) || !state->focused) return COLOSSUS_CEF_DENIED;
  if (!input || input->version != COLOSSUS_CEF_PRESENTATION_VERSION ||
      input->modifiers & ~uint32_t(0x1fff)) return COLOSSUS_CEF_INVALID;
  auto host = browser->browser->GetHost();
  // Presentation input uses physical frame pixels; CEF consumes logical DIP.
  CefMouseEvent mouse; mouse.modifiers = input->modifiers;
  if (input->kind >= COLOSSUS_CEF_MOUSE_MOVE && input->kind <= COLOSSUS_CEF_MOUSE_WHEEL) {
    if (input->x < 0 || input->y < 0 || input->x >= std::ceil(state->width * state->scale) ||
        input->y >= std::ceil(state->height * state->scale))
      return COLOSSUS_CEF_INVALID;
    mouse.x = static_cast<int32_t>(input->x / state->scale);
    mouse.y = static_cast<int32_t>(input->y / state->scale);
    switch (input->kind) {
      case COLOSSUS_CEF_MOUSE_MOVE: host->SendMouseMoveEvent(mouse, false); break;
      case COLOSSUS_CEF_MOUSE_DOWN: case COLOSSUS_CEF_MOUSE_UP:
        if (input->button < 0 || input->button > 2) return COLOSSUS_CEF_INVALID;
        host->SendMouseClickEvent(mouse, static_cast<cef_mouse_button_type_t>(input->button),
                                 input->kind == COLOSSUS_CEF_MOUSE_UP, 1); break;
      default:
        if (input->wheel_x < -4096 || input->wheel_x > 4096 || input->wheel_y < -4096 || input->wheel_y > 4096)
          return COLOSSUS_CEF_INVALID;
        host->SendMouseWheelEvent(mouse, input->wheel_x, input->wheel_y); break;
    }
    return COLOSSUS_CEF_OK;
  }
  if (input->kind == COLOSSUS_CEF_IME_CANCEL) { host->ImeCancelComposition(); return COLOSSUS_CEF_OK; }
  if (input->kind == COLOSSUS_CEF_IME_COMMIT) {
    if (!Utf16(input->text, input->text_units)) return COLOSSUS_CEF_INVALID;
    const std::u16string text(input->text, input->text + input->text_units);
    host->ImeCommitText(CefString(text), CefRange(-1, -1), 0);
    return COLOSSUS_CEF_OK;
  }
  if (input->kind < COLOSSUS_CEF_KEY_DOWN || input->kind > COLOSSUS_CEF_CHARACTER ||
      input->key_code <= 0 || input->key_code > 65535 ||
      (input->kind == COLOSSUS_CEF_CHARACTER && input->key_code >= 0xd800 && input->key_code <= 0xdfff)) return COLOSSUS_CEF_INVALID;
  CefKeyEvent key; key.type = input->kind == COLOSSUS_CEF_KEY_DOWN ? KEYEVENT_RAWKEYDOWN :
      input->kind == COLOSSUS_CEF_KEY_UP ? KEYEVENT_KEYUP : KEYEVENT_CHAR;
  key.windows_key_code = input->key_code; key.character = input->key_code;
  key.unmodified_character = input->key_code; key.modifiers = input->modifiers;
  host->SendKeyEvent(key);
  return COLOSSUS_CEF_OK;
}
