#ifndef COLOSSUS_MAC_NETWORK_ENVELOPE_H_
#define COLOSSUS_MAC_NETWORK_ENVELOPE_H_
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

/* Source construction is not proof of application or production acceptance.
 * The same implementation is compiled as C for the libSystem-only front. */
enum colossus_mac_network_role {
  COLOSSUS_MAC_ROLE_RENDERER, COLOSSUS_MAC_ROLE_GPU, COLOSSUS_MAC_ROLE_NETWORK,
  COLOSSUS_MAC_ROLE_UTILITY, COLOSSUS_MAC_ROLE_AUDIO, COLOSSUS_MAC_ROLE_CDM,
  COLOSSUS_MAC_ROLE_MIRRORING, COLOSSUS_MAC_ROLE_PRINT_BACKEND,
  COLOSSUS_MAC_ROLE_PRINT_COMPOSITOR, COLOSSUS_MAC_ROLE_PROXY_RESOLVER,
  COLOSSUS_MAC_ROLE_SCREEN_AI, COLOSSUS_MAC_ROLE_SPEECH_RECOGNITION,
  COLOSSUS_MAC_ROLE_ON_DEVICE_MODEL, COLOSSUS_MAC_ROLE_ON_DEVICE_TRANSLATION,
  COLOSSUS_MAC_ROLE_WEBNN
};
struct colossus_mac_network_binding {
  uint32_t audit_session;
  uint16_t proxy_port;
  const char* allocation_root;
  const char* bundle_path;
  const char* bundle_id;
  const char* profile_root;
  const char* broker_root;
  const char* personal_home;
};
/* All paths are canonical native-owned absolute paths. Policy resources belong
 * to the retained, sealed allocation; argv never supplies these values.
 * Returned source is bounded to 128 KiB and must be freed by the caller.
 * Parameters are represented by definitions of a closed local param function;
 * no browser-supplied policy text, compiled blob or parameter is accepted.
 * ScreenAI, SODA and TranslateKit roles fail closed until their downloaded
 * native components have a separate publisher-bound enrollment contract.
 * WebNN remains rejected until its extra platform parameter is verified. */
int colossus_mac_helper_policy(enum colossus_mac_network_role role,
    const struct colossus_mac_network_binding* binding, const char* executable,
    int32_t browser_pid, uint32_t os_version, char** source);
/* Main profile permits only the listed fixed sealed fronts to leave its profile.
 * Each front applies its own composite before executing its fixed body.
 * No other process-exec or process-fork receives authority. */
int colossus_mac_main_network_policy(const struct colossus_mac_network_binding* binding,
    const char* const* fronts, size_t count, char** source);
/* Minimal front permits only an exact fixed body exec, with profile inheritance. */
int colossus_mac_helper_body_policy(enum colossus_mac_network_role role,
    const struct colossus_mac_network_binding* binding, const char* executable,
    const char* body, int32_t browser_pid, uint32_t os_version, char** source);
/* Parse the sealed fixed policy resource's closed line format. Exactly eight
 * fields follow the marker: ASID, port, allocation, bundle, bundle id, profile,
 * broker and personal home. Storage owns the borrowed strings until application. */
int colossus_mac_network_binding_parse(char* storage, size_t size,
    struct colossus_mac_network_binding* binding);
const char* colossus_mac_role_policy_sha256(enum colossus_mac_network_role role);
#ifdef __cplusplus
}
#endif
#endif
