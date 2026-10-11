#ifndef COLOSSUS_MAC_HELPER_PROGRAMS_H_
#define COLOSSUS_MAC_HELPER_PROGRAMS_H_
#include <array>
#include <optional>
#include <string>
#include <string_view>

namespace colossus {
// Internal native paths, prepared from the signed component's supplied base
// executable. Never an ABI, command, renderer/model input or arbitrary setter.
struct MacHelperPrograms {
  std::array<std::string, 5> paths;
  std::optional<std::string> GpuProgram(std::string_view process,
                                        std::string_view current) const;
};
std::optional<MacHelperPrograms> PrepareMacHelperPrograms(const char* base);
int MacChildProcessClass(std::string_view process);
int MacHelperProgramKind(std::string_view program);
}  // namespace colossus
#endif
