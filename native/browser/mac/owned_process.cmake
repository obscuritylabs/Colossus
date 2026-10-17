# Included only after actual patched CEF export/receipt verification. No target
# here executes a front, invokes Security APIs, or enables production discovery.
add_library(colossus_mac_owned_policy STATIC
  process-broker/fixed_policy.c process-broker/policy_transition.c
  process-broker/custody_policy.c process-broker/custody_probe.c process-broker/image_verify.c)
target_link_libraries(colossus_mac_owned_policy PUBLIC colossus_mac_network_policy)
target_include_directories(colossus_mac_owned_policy PUBLIC process-broker)
target_compile_options(colossus_mac_owned_policy PRIVATE -Wall -Wextra -Werror)
add_executable(colossus-mac-custody-policy-source-test EXCLUDE_FROM_ALL
  process-broker/tests/custody_policy.c process-broker/custody_policy.c)
target_compile_options(colossus-mac-custody-policy-source-test PRIVATE -Wall -Wextra -Werror)
add_executable(colossus-mac-custody-roles-source-test EXCLUDE_FROM_ALL
  process-broker/tests/custody_roles.c)
target_link_libraries(colossus-mac-custody-roles-source-test PRIVATE colossus_mac_owned_policy)
target_compile_options(colossus-mac-custody-roles-source-test PRIVATE -Wall -Wextra -Werror)
add_executable(colossus-mac-custody-role-denial-test EXCLUDE_FROM_ALL
  process-broker/tests/custody_denials.c)
target_link_libraries(colossus-mac-custody-role-denial-test PRIVATE colossus_mac_owned_policy sandbox)
target_compile_options(colossus-mac-custody-role-denial-test PRIVATE -Wall -Wextra -Werror)

function(colossus_add_owned_helper target name plist)
  add_executable(${target} MACOSX_BUNDLE process-broker/front_loader.c)
  target_link_libraries(${target} PRIVATE colossus_mac_owned_policy sandbox)
  target_compile_options(${target} PRIVATE -Wall -Wextra -Werror)
  set_target_properties(${target} PROPERTIES OUTPUT_NAME "${name}"
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}/helpers"
    MACOSX_BUNDLE_INFO_PLIST "${plist}")
  set(body "${target}-body")
  add_library(${body} SHARED process-broker/helper_body.mm)
  SET_LIBRARY_TARGET_PROPERTIES(${body})
  target_include_directories(${body} SYSTEM PRIVATE "${CEF_ROOT}")
  target_link_libraries(${body} PRIVATE libcef_dll_wrapper ${CEF_STANDARD_LIBS} sandbox)
  target_compile_options(${body} PRIVATE -Wall -Wextra -Werror)
  set_target_properties(${body} PROPERTIES PREFIX "" SUFFIX ".dylib"
    OUTPUT_NAME "${name} Body" MACOSX_RPATH ON INSTALL_NAME_DIR "@loader_path"
    LIBRARY_OUTPUT_DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}/helpers/${name}.app/Contents/MacOS")
  add_dependencies(${body} ${target})
  install(TARGETS ${target} BUNDLE DESTINATION helpers)
  install(TARGETS ${body} LIBRARY DESTINATION "helpers/${name}.app/Contents/MacOS")
  set_property(GLOBAL APPEND PROPERTY COLOSSUS_OWNED_PROCESS_NATIVE_TARGETS ${target} ${body})
endfunction()

function(colossus_add_owned_artifact_receipt)
  get_property(_owned_helper_targets GLOBAL PROPERTY COLOSSUS_OWNED_PROCESS_NATIVE_TARGETS)
  list(LENGTH _owned_helper_targets _owned_helper_count)
  if(NOT _owned_helper_count EQUAL 10)
    message(FATAL_ERROR "Owned native artifact receipt requires exactly five helper fronts and bodies.")
  endif()
  add_custom_target(colossus-mac-owned-process-artifacts ALL
    COMMAND "${Python3_EXECUTABLE}" -B
      "${CMAKE_CURRENT_SOURCE_DIR}/scripts/owned_process_component.py"
      --cef-root "${CEF_ROOT}" --platform "${_owned_platform}"
      --write-native-build "${CMAKE_CURRENT_BINARY_DIR}"
    BYPRODUCTS "${CMAKE_CURRENT_BINARY_DIR}/owned-process-native-artifacts.json"
    VERBATIM)
  # Target ordering avoids Makefile archive-member interpretation of helper
  # executable names ending in parentheses. The command still verifies every
  # actual completed artifact before emitting the receipt.
  add_dependencies(colossus-mac-owned-process-artifacts
    colossus_cef colossus-mac-browser-envelope-front ${_owned_helper_targets})
endfunction()
