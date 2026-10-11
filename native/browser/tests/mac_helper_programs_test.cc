// File-only native helper selection checks. No CEF load/process/AppKit/Keychain.
#include "mac_helper_programs.h"
#include <cassert>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <sys/stat.h>
#include <unistd.h>

namespace fs = std::filesystem;
int main() {
  using namespace colossus;
  assert(!PrepareMacHelperPrograms(nullptr));
  assert(!PrepareMacHelperPrograms("relative"));
  char root[] = "/private/tmp/colossus-helper-paths-XXXXXX";
  assert(mkdtemp(root));
  const fs::path bundle = fs::path(root) / "Browser.app";
  const char* suffixes[] = {"", " (Renderer)", " (GPU)", " (Plugin)", " (Alerts)"};
  std::array<std::string, 5> paths;
  for (size_t i = 0; i < paths.size(); ++i) {
    const std::string name = std::string("Colossus Browser Helper") + suffixes[i];
    const fs::path program = bundle / "Contents/Frameworks" / (name + ".app") /
      "Contents/MacOS" / name;
    fs::create_directories(program.parent_path());
    std::ofstream(program).put('x');
    assert(chmod(program.c_str(), 0700) == 0);
    paths[i] = program.string();
  }
  auto owned = PrepareMacHelperPrograms(paths[0].c_str());
  assert(owned && owned->paths == paths);
  assert(owned->GpuProgram("gpu-process", paths[0]) == paths[2]);
  assert(owned->GpuProgram("gpu-process", paths[2]) == paths[2]);
  assert(!owned->GpuProgram("renderer", paths[0]));
  assert(!owned->GpuProgram("utility", paths[0]));
  assert(!owned->GpuProgram("gpu-process", paths[1]));
  assert(!owned->GpuProgram("gpu-process", "/bin/sh"));
  assert(!PrepareMacHelperPrograms(paths[2].c_str()));
  assert(chmod(paths[2].c_str(), 0777) == 0);
  assert(!PrepareMacHelperPrograms(paths[0].c_str()));
  assert(chmod(paths[2].c_str(), 0700) == 0);
  const std::string alternate = paths[2] + " Alternate";
  fs::rename(paths[2], alternate);
  fs::create_symlink(alternate, paths[2]);
  assert(!PrepareMacHelperPrograms(paths[0].c_str()));
  fs::remove(paths[2]); fs::rename(alternate, paths[2]);
  fs::remove(paths[4]);
  assert(!PrepareMacHelperPrograms(paths[0].c_str()));
  assert(MacChildProcessClass("gpu-process") == 2);
  assert(MacChildProcessClass("utility") == 3);
  assert(MacChildProcessClass("unexpected") == 0);
  assert(MacHelperProgramKind(paths[2]) == 2);
  assert(MacHelperProgramKind("/bin/sh") == 9);
  fs::remove_all(root);
  assert(!fs::exists(root));
  std::puts("PASS fixed canonical helper selection; mismatched role/program, unsafe/missing/symlink component rejected; owned files removed");
}
