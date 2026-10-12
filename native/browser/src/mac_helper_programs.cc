#include "mac_helper_programs.h"
#include <cstring>
#include <filesystem>
#include <limits.h>
#include <sys/stat.h>
#include <unistd.h>

namespace colossus {
namespace {
constexpr std::string_view kBase =
  "/Contents/Frameworks/Colossus Browser Helper.app/Contents/MacOS/Colossus Browser Helper";
constexpr std::array<const char*, 5> kSuffixes = {"", " (Renderer)", " (GPU)", " (Plugin)", " (Alerts)"};

bool Executable(const std::string& path) {
  if (path.size() >= PATH_MAX) return false;
  std::error_code error;
  const auto canonical = std::filesystem::canonical(path, error);
  if (error || canonical.string() != path) return false;
  struct stat metadata;
  return lstat(path.c_str(), &metadata) == 0 && S_ISREG(metadata.st_mode) &&
    metadata.st_uid == geteuid() && metadata.st_nlink == 1 &&
    (metadata.st_mode & 0111) != 0 && (metadata.st_mode & 0022) == 0;
}
}  // namespace

std::optional<MacHelperPrograms> PrepareMacHelperPrograms(const char* base) {
  if (!base) return std::nullopt;
  const size_t size = strnlen(base, PATH_MAX);
  if (size >= PATH_MAX || size <= kBase.size()) return std::nullopt;
  const std::string path(base, size);
  if (!path.ends_with(kBase) || path.front() != '/') return std::nullopt;
  const std::string bundle = path.substr(0, path.size() - kBase.size());
  MacHelperPrograms result;
  for (size_t i = 0; i < kSuffixes.size(); ++i) {
    const std::string name = std::string("Colossus Browser Helper") + kSuffixes[i];
    result.paths[i] = bundle + "/Contents/Frameworks/" + name + ".app/Contents/MacOS/" + name;
    if (!Executable(result.paths[i])) return std::nullopt;
  }
  return result;
}

std::optional<std::string> MacHelperPrograms::Program(std::string_view process) const {
  if (process == "renderer") return paths[1];
  if (process == "gpu-process") return paths[2];
  if (process == "utility") return paths[0];
  return std::nullopt;
}

int MacChildProcessClass(std::string_view process) {
  if (process == "renderer") return 1;
  if (process == "gpu-process") return 2;
  if (process == "utility") return 3;
  return 0;
}

int MacHelperProgramKind(std::string_view program) {
  const auto slash = program.rfind('/');
  const auto basename = slash == std::string_view::npos ? program : program.substr(slash + 1);
  for (size_t i = 0; i < kSuffixes.size(); ++i)
    if (basename == std::string("Colossus Browser Helper") + kSuffixes[i]) return static_cast<int>(i);
  return 9;
}
}  // namespace colossus
