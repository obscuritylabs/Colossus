# Include after find_package(CEF). Invocation remains explicit: ordinary preview
# staging has no sealed per-allocation policy resource and uses its normal helpers.
find_package(Python3 REQUIRED COMPONENTS Interpreter)
set(_network_generated "${CMAKE_CURRENT_BINARY_DIR}/network-generated")
file(MAKE_DIRECTORY "${_network_generated}")
file(GLOB _network_policy_sources CONFIGURE_DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/mac/sandbox/*.sb")
add_custom_command(
  OUTPUT "${_network_generated}/network_policies.generated.h"
  COMMAND "${Python3_EXECUTABLE}" -B "${CMAKE_CURRENT_SOURCE_DIR}/scripts/mac_network_policies.py"
    --source "${CMAKE_CURRENT_SOURCE_DIR}/mac/sandbox"
    --output "${_network_generated}/network_policies.generated.h"
  DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/scripts/mac_network_policies.py"
    "${CMAKE_CURRENT_SOURCE_DIR}/mac/sandbox/policies.lock.json" ${_network_policy_sources}
  VERBATIM)
# The .cc implementation is intentionally valid C. Fronts must have no libc++
# dependency and no framework initializer before their first sandbox application.
set_source_files_properties(mac/network_envelope.cc PROPERTIES LANGUAGE C)
add_library(colossus_mac_network_policy STATIC mac/network_envelope.cc
  "${_network_generated}/network_policies.generated.h")
target_include_directories(colossus_mac_network_policy PUBLIC mac PRIVATE "${_network_generated}")
target_compile_options(colossus_mac_network_policy PRIVATE -Wall -Wextra -Werror)

function(colossus_add_network_helper target name plist kind)
  add_executable(${target} MACOSX_BUNDLE mac/helper_envelope_front.c)
  target_compile_definitions(${target} PRIVATE COLOSSUS_MAC_HELPER_KIND=${kind})
  target_link_libraries(${target} PRIVATE colossus_mac_network_policy sandbox)
  target_compile_options(${target} PRIVATE -Wall -Wextra -Werror)
  set_target_properties(${target} PROPERTIES OUTPUT_NAME "${name}"
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}/helpers"
    MACOSX_BUNDLE_INFO_PLIST "${plist}")

  set(body "${target}-body")
  add_executable(${body} mac/helper_envelope_body.mm)
  SET_EXECUTABLE_TARGET_PROPERTIES(${body})
  target_include_directories(${body} SYSTEM PRIVATE "${CEF_ROOT}")
  target_link_libraries(${body} PRIVATE libcef_dll_wrapper ${CEF_STANDARD_LIBS})
  target_compile_options(${body} PRIVATE -Wall -Wextra -Werror)
  set_target_properties(${body} PROPERTIES OUTPUT_NAME "${name} Body"
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}/helpers/${name}.app/Contents/MacOS")
  add_dependencies(${body} ${target})
  install(TARGETS ${target} BUNDLE DESTINATION helpers)
  install(TARGETS ${body} RUNTIME DESTINATION "helpers/${name}.app/Contents/MacOS")
endfunction()

add_executable(colossus-mac-network-envelope-source-test EXCLUDE_FROM_ALL tests/mac_network_envelope_test.c)
target_link_libraries(colossus-mac-network-envelope-source-test PRIVATE colossus_mac_network_policy)
target_compile_options(colossus-mac-network-envelope-source-test PRIVATE -Wall -Wextra -Werror)
add_executable(colossus-mac-network-envelope-compile-test EXCLUDE_FROM_ALL tests/mac_network_compile_test.c)
target_link_libraries(colossus-mac-network-envelope-compile-test PRIVATE colossus_mac_network_policy sandbox)
target_compile_options(colossus-mac-network-envelope-compile-test PRIVATE -Wall -Wextra -Werror)
add_executable(colossus-mac-network-denial-test EXCLUDE_FROM_ALL tests/mac_network_denial_test.c)
target_link_libraries(colossus-mac-network-denial-test PRIVATE colossus_mac_network_policy sandbox)
target_compile_options(colossus-mac-network-denial-test PRIVATE -Wall -Wextra -Werror)
