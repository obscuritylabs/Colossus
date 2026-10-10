#ifndef COLOSSUS_CEF_CUSTODY_LINUX_H
#define COLOSSUS_CEF_CUSTODY_LINUX_H
#include "colossus_cef.h"
#include "include/cef_command_line.h"
#include "include/cef_process_message.h"
#include "include/cef_render_process_handler.h"
namespace colossus {
CefRefPtr<CefRenderProcessHandler> CustodyRenderer();
void CustodyChild(CefRefPtr<CefCommandLine> line);
void CustodyDocument(colossus_cef_tab tab, uint64_t generation, CefRefPtr<CefFrame> frame);
bool CustodyMessage(colossus_cef_tab tab, uint64_t generation,
  CefRefPtr<CefBrowser> browser, CefRefPtr<CefFrame> frame,
  CefProcessId process, CefRefPtr<CefProcessMessage> message);
void CustodyFinish();
}
// Defined only by the OFF-default Linux experiment build. Native bootstrap only.
extern "C" int32_t colossus_cef_custody_test_prepare(const char* fixture_url);
#endif
