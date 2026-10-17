/* Compiler-only native syntax acceptance. No sandbox application, spawning,
 * Keychain access, CEF loading, or audit-port operations occur here. */
#include "network_envelope.h"
#include <sandbox.h>
#include <stdio.h>
#include <stdlib.h>

typedef struct sandbox_params_t sandbox_params_t;
typedef struct sandbox_profile_t sandbox_profile_t;
extern sandbox_params_t *sandbox_create_params(void);
extern void sandbox_free_params(sandbox_params_t *);
extern sandbox_profile_t *sandbox_compile_string(const char *, sandbox_params_t *,
                                                  char **);
extern void sandbox_free_profile(sandbox_profile_t *);

#define BUNDLE "/private/tmp/colossus-network-compile/Browser.app"
#define FRONT(s) BUNDLE "/Contents/Frameworks/Colossus Browser Helper" s \
    ".app/Contents/MacOS/Colossus Browser Helper" s

static int compile(const char *source, sandbox_params_t *params) {
  char *error = NULL;
  sandbox_profile_t *profile = sandbox_compile_string(source, params, &error);
  if (profile) sandbox_free_profile(profile);
  if (error) {
    fprintf(stderr, "sandbox policy compilation failed: %s\n", error);
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
    sandbox_free_error(error);
#pragma clang diagnostic pop
  }
  return profile != NULL;
}

int main(void) {
  const struct colossus_mac_network_binding binding = {
      42, 54321, "/private/tmp/colossus-network-compile", BUNDLE,
      "com.obscuritylabs.colossus.native-browser-host.preview",
      "/private/tmp/colossus-network-compile/profile",
      "/private/tmp/colossus-network-broker", "/Users/fixture"};
  const char *const fronts[] = {FRONT(""), FRONT(" (Alerts)"), FRONT(" (GPU)"),
                                FRONT(" (Plugin)"), FRONT(" (Renderer)")};
  sandbox_params_t *params = sandbox_create_params();
  if (!params) return 1;
  char *source = NULL;
  if (!colossus_mac_main_network_policy(&binding, fronts, 5, &source)) return 2;
  int valid = compile(source, params);
  free(source);
  for (int role = COLOSSUS_MAC_ROLE_RENDERER; valid && role <= COLOSSUS_MAC_ROLE_WEBNN;
       ++role) {
    if (role == COLOSSUS_MAC_ROLE_SCREEN_AI ||
        role == COLOSSUS_MAC_ROLE_SPEECH_RECOGNITION ||
        role == COLOSSUS_MAC_ROLE_ON_DEVICE_TRANSLATION ||
        role == COLOSSUS_MAC_ROLE_WEBNN) continue;
    const char *front = role == COLOSSUS_MAC_ROLE_RENDERER ? fronts[4] :
                        role == COLOSSUS_MAC_ROLE_GPU ? fronts[2] :
                        role == COLOSSUS_MAC_ROLE_CDM ? fronts[3] : fronts[0];
    char body[4096];
    if (snprintf(body, sizeof(body), "%s Body", front) >= (int)sizeof(body) ||
        !colossus_mac_helper_body_policy((enum colossus_mac_network_role)role,
                                         &binding, front, body, 123, 2600, &source)) {
      valid = 0;
      break;
    }
    valid = compile(source, params);
    free(source);
  }
  sandbox_free_params(params);
  if (valid) puts("macOS network policies compile without application");
  return valid ? 0 : 3;
}
