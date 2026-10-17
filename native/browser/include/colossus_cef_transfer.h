#ifndef COLOSSUS_CEF_TRANSFER_H
#define COLOSSUS_CEF_TRANSFER_H

#include "colossus_cef.h"

#ifdef __cplusplus
extern "C" {
#endif

#define COLOSSUS_CEF_TRANSFER_VERSION 1u
#define COLOSSUS_CEF_MAX_DOWNLOAD_BYTES (4u * 1024u * 1024u)
#define COLOSSUS_CEF_MAX_TRANSFER_URL 4096u

enum colossus_cef_download_status {
  COLOSSUS_CEF_DOWNLOAD_ARMED = 0,
  COLOSSUS_CEF_DOWNLOAD_WRITING = 1,
  COLOSSUS_CEF_DOWNLOAD_COMPLETE = 2,
  COLOSSUS_CEF_DOWNLOAD_CANCELLED = 3,
  COLOSSUS_CEF_DOWNLOAD_FAILED = 4
};

typedef struct colossus_cef_download_state {
  uint32_t status;
  uint64_t received_bytes;
  uint64_t total_bytes;
  char final_url[COLOSSUS_CEF_MAX_TRANSFER_URL + 1u];
  uint32_t final_url_len;
} colossus_cef_download_state;

/* Private supervisor/permit-consumer ABI, always on the CEF UI thread. One
 * intent per owned tab. nonce is exactly 32 lowercase hexadecimal bytes plus
 * NUL; trusted_path is an exclusive, generated file in the host's private
 * transfer directory, never a page/model supplied filename. expected_url is
 * the complete original HTTP(S) URL. Each callback rechecks tab, native document,
 * URL envelope, deadline and byte ceiling. The page's suggested name is ignored.
 * No save dialog or default download directory is ever used.
 *
 * Cancellation requests do not prove writer retirement: WRITING persists until
 * CEF reports cancellation, interruption or completion. Retain the private file
 * and its owning directory through CefShutdown when quiescence is uncertain.
 * No path, filename, raw interrupt reason or file bytes cross the poll boundary.
 */
int32_t colossus_cef_download_arm(colossus_cef_tab tab, uint64_t generation,
  uint64_t expected_document, const char* nonce, const char* trusted_path,
  const char* expected_url, uint64_t max_bytes, uint32_t ttl_ms);
/* Starts only the cached, native-validated link URL, using GET. There is no URL
 * or path argument, no page click handler, and no navigation of the document.
 * Page-initiated downloads remain blocked even while an intent is armed.
 */
int32_t colossus_cef_download_start(colossus_cef_tab tab, uint64_t generation,
  uint64_t expected_document, const char* nonce);
int32_t colossus_cef_download_poll(colossus_cef_tab tab, uint64_t generation,
  uint64_t expected_document, const char* nonce,
  colossus_cef_download_state* result);
int32_t colossus_cef_download_cancel(colossus_cef_tab tab, uint64_t generation,
  uint64_t expected_document, const char* nonce);

#ifdef __cplusplus
}
#endif
#endif
