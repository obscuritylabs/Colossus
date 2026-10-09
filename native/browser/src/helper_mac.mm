#include "include/cef_app.h"
#include "include/cef_sandbox_mac.h"
#include "include/wrapper/cef_library_loader.h"

int main(int argc, char** argv) {
  // This entry must run before loading Chromium or creating AppKit objects.
  CefScopedSandboxContext sandbox;
  if (!sandbox.Initialize(argc, argv)) return 1;
  CefScopedLibraryLoader library;
  if (!library.LoadInHelper()) return 2;
  return CefExecuteProcess(CefMainArgs(argc, argv), nullptr, nullptr);
}
