#include "host_internal.h"
#include <cerrno>
#include <filesystem>
#if !defined(OS_WIN)
#include <sys/stat.h>
#include <unistd.h>
#endif

namespace colossus {
namespace {
bool persistent = false;
uint64_t persistent_context = 0;
std::string context_path;
}
bool ProfileRoot(const char* root) {
  if (!persistent) return true;
  if (!root || !*root) return false;
  std::error_code error;
  // char8_t input retains UTF-8 conversion on Windows without the deprecated
  // C++17 u8path helper. Native bootstrap supplies this bounded UTF-8 path.
  const std::string utf8(root);
  const auto base = std::filesystem::path(std::u8string(utf8.begin(), utf8.end()));
  const auto path = base / "context";
  if (!base.is_absolute() || std::filesystem::canonical(base, error) != base || error) return false;
#if defined(OS_WIN)
  // The retained native-store lease already verified the private root's ACL.
  // Windows inherits that ACL when creating this fixed child directory.
  std::filesystem::create_directory(path, error);
  if (error) return false;
#else
  if (::mkdir(path.c_str(), 0700) != 0 && errno != EEXIST) return false;
  struct stat metadata{};
  if (::lstat(path.c_str(), &metadata) != 0 || !S_ISDIR(metadata.st_mode) ||
      metadata.st_uid != ::geteuid() || (metadata.st_mode & 0077) != 0) return false;
#endif
  const auto status = std::filesystem::symlink_status(path, error);
  if (error || !std::filesystem::is_directory(status) || std::filesystem::is_symlink(status) ||
      std::filesystem::canonical(path, error) != path || error) return false;
  const auto bytes = path.generic_u8string();
  context_path.assign(bytes.begin(), bytes.end());
  return true;
}
int32_t ProfileContext(uint64_t owner, std::string* cache) {
  cache->clear();
  if (!persistent) return COLOSSUS_CEF_OK;
  if (!owner || context_path.empty()) return COLOSSUS_CEF_UNAVAILABLE;
  if (persistent_context && persistent_context != owner) return COLOSSUS_CEF_DENIED;
  persistent_context = owner;
  *cache = context_path;
  return COLOSSUS_CEF_OK;
}
void ProfileShutdown() {
  persistent_context = 0; context_path.clear(); persistent = false;
}
}
extern "C" int32_t colossus_cef_profile_persistent(int32_t enabled) {
  if (colossus::initialized) return COLOSSUS_CEF_BUSY;
  if (enabled != 0 && enabled != 1) return COLOSSUS_CEF_INVALID;
  colossus::persistent = enabled != 0;
  return COLOSSUS_CEF_OK;
}
