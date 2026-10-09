#include "colossus_cef.h"
#include "include/cef_parser.h"
#include <chrono>
#include <cstdio>
#include <filesystem>
#include <thread>
#include <string>

namespace {
bool created = false, result_received = false, closed = false;
bool navigation_complete = false;
int32_t result_success = 0;
int32_t expected_command = 0;
std::string result_json;
std::string fixture_origin;
void Event(void*, uint64_t, uint64_t, uint32_t event, int32_t command,
           int32_t success, const uint8_t* payload, size_t size) {
  if (event == COLOSSUS_CEF_CREATED) created = true;
  if (event == COLOSSUS_CEF_CLOSED_EVENT) closed = true;
  if (event == COLOSSUS_CEF_LOADING && !success) navigation_complete = true;
  if (event == COLOSSUS_CEF_DEVTOOLS_RESULT && command == expected_command) {
    result_received = true; result_success = success;
    result_json.assign(payload ? reinterpret_cast<const char*>(payload) : "", size);
    std::printf("devtools_command=%d success=%d bounded_bytes=%zu\n", command, success, size);
  }
}
int32_t Allow(void*, uint64_t, uint64_t, const char* bytes, size_t size, int32_t) {
  if (fixture_origin.empty()) return 0;
  std::string url(bytes, size);
  return url.starts_with(fixture_origin + "/");
}
bool PumpUntil(bool& condition) {
  auto until = std::chrono::steady_clock::now() + std::chrono::seconds(10);
  while (!condition && std::chrono::steady_clock::now() < until) {
    if (colossus_cef_pump() != COLOSSUS_CEF_OK) return false;
    std::this_thread::sleep_for(std::chrono::milliseconds(5));
  }
  return condition;
}
bool Method(int32_t id, const char* method, const std::string& params) {
  expected_command = id; result_received = false; result_success = 0; result_json.clear();
  return colossus_cef_devtools(1, 1, id, method,
    reinterpret_cast<const uint8_t*>(params.data()), params.size()) == COLOSSUS_CEF_OK &&
    PumpUntil(result_received) && result_success;
}
int NodeId(const std::string& json, const char* key) {
  auto parsed = CefParseJSON(json, JSON_PARSER_RFC);
  if (!parsed || parsed->GetType() != VTYPE_DICTIONARY) return 0;
  auto dictionary = parsed->GetDictionary();
  if (std::string(key) == "root") {
    auto root = dictionary->GetDictionary("root"); return root ? root->GetInt("nodeId") : 0;
  }
  return dictionary->GetInt(key);
}
bool Fixture(const std::string& url) {
  navigation_complete = false;
  if (colossus_cef_navigate(1, 1, url.c_str()) || !PumpUntil(navigation_complete)) return false;
  if (!Method(2, "DOM.getDocument", "{\"depth\":-1,\"pierce\":true}")) return false;
  int root = NodeId(result_json, "root");
  if (!root || result_json.find("Colossus native browser fixture") == std::string::npos) return false;
  if (!Method(3, "DOM.querySelector", "{\"nodeId\":" + std::to_string(root) + ",\"selector\":\"#continue\"}")) return false;
  int button = NodeId(result_json, "nodeId");
  if (!button || !Method(4, "DOM.getBoxModel", "{\"nodeId\":" + std::to_string(button) + "}")) return false;
  auto parsed = CefParseJSON(result_json, JSON_PARSER_RFC);
  auto model = parsed->GetDictionary()->GetDictionary("model");
  auto quad = model ? model->GetList("content") : nullptr;
  if (!quad || quad->GetSize() != 8) return false;
  auto number = [&quad](size_t index) {
    return quad->GetType(index) == VTYPE_INT ? double(quad->GetInt(index)) : quad->GetDouble(index);
  };
  auto x = (number(0) + number(2)) / 2;
  auto y = (number(1) + number(5)) / 2;
  auto coordinates = "\"x\":" + std::to_string(x) + ",\"y\":" + std::to_string(y);
  if (!Method(5, "Input.dispatchMouseEvent", "{\"type\":\"mousePressed\",\"button\":\"left\",\"clickCount\":1," + coordinates + "}")) return false;
  if (!Method(6, "Input.dispatchMouseEvent", "{\"type\":\"mouseReleased\",\"button\":\"left\",\"clickCount\":1," + coordinates + "}")) return false;
  if (!Method(7, "DOM.getDocument", "{\"depth\":-1}")) return false;
  if (result_json.find("Native click received") == std::string::npos) return false;
  if (!Method(8, "Page.captureScreenshot", "{\"format\":\"png\"}")) return false;
  parsed = CefParseJSON(result_json, JSON_PARSER_RFC);
  auto png = CefBase64Decode(parsed->GetDictionary()->GetString("data"));
  uint8_t signature[8]{};
  if (!png || png->GetSize() < 24 || png->GetData(signature, 8, 0) != 8) return false;
  static const uint8_t expected[] = {137,80,78,71,13,10,26,10};
  for (size_t i = 0; i < 8; ++i) if (signature[i] != expected[i]) return false;
  std::printf("native_fixture=passed png_bytes=%zu\n", png->GetSize());
  return true;
}
}

int main(int argc, char** argv) {
  colossus_cef_bootstrap_options o{};
  o.abi_version = COLOSSUS_CEF_ABI_VERSION; o.argc = argc; o.argv = argv;
  std::string fixture;
  for (int i = 1; i < argc; ++i) {
    std::string arg = argv[i];
    if (arg.starts_with("--fixture-url=")) fixture = arg.substr(14);
  }
  if (!fixture.empty()) {
    auto slash = fixture.find('/', 7);
    if (!fixture.starts_with("http://127.0.0.1:") || slash == std::string::npos) return 7;
    fixture_origin = fixture.substr(0, slash);
  }
  auto root = std::filesystem::absolute("probe-profile").string();
  o.root_cache_path = root.c_str(); o.headless = 1;
  o.callbacks.event = Event; o.callbacks.allow_url = Allow;
  int32_t subprocess = -1;
  const auto status = colossus_cef_bootstrap(&o, &subprocess);
  if (subprocess >= 0) return subprocess;
  std::fprintf(stderr, "probe_bootstrap_status=%d\n", status);
  if (status) { std::fprintf(stderr, "bootstrap_status=%d\n", status); return 2; }
  auto create = colossus_cef_create(1, 1, 1, 0, {0, 0, 1024, 768}, "about:blank");
  std::fprintf(stderr, "probe_create_status=%d\n", create);
  if (create || !PumpUntil(created)) return 3;
  if (!Method(1, "Page.getFrameTree", "{}")) return 4;
  if (colossus_cef_navigate(1, 2, "about:blank") != COLOSSUS_CEF_CLOSED ||
      colossus_cef_navigate(1, 1, "https://denied.invalid/") != COLOSSUS_CEF_DENIED ||
      colossus_cef_devtools(1, 1, 9, "Page.getFrameTree", reinterpret_cast<const uint8_t*>("[]"), 2)
        != COLOSSUS_CEF_INVALID) return 8;
  if (!fixture.empty() && !Fixture(fixture)) return 9;
  std::printf("native_negative_controls=passed\n");
  if (colossus_cef_close(1, 1) || !PumpUntil(closed) || colossus_cef_shutdown()) return 5;
  return result_success ? 0 : 6;
}
