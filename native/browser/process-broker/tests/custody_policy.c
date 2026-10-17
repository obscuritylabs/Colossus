/* Pure policy regression; no process launch, Seatbelt, Security or CEF calls. */
#include "../custody_policy.h"
#include <assert.h>
#include <stdlib.h>
#include <string.h>
int main(void) {
  const char* source="(version 1)\n(allow default)\n"
    "(allow file-read* file-write* (literal \"/private/custody/nested/keychain\") (subpath \"/private\") (with report))\n"
    "(allow file-read-data)\n(allow file-issue-extension (extension-class \"com.apple.app-sandbox.read\"))\n"
    "(if (param \"debug\") (allow file* (extension \"com.apple.app-sandbox.read-write\")) #f)\n"
    "(allow mach-lookup (global-name \"(allow file-read*)\"))\n"
    "(allow mach-lookup (global-name \"com.apple.SecurityServer\") (xpc-service-name \"com.apple.securityd.xpc\"))\n";
  char* output=NULL;assert(colossus_owned_custody_policy(source,strlen(source),"/private/custody",&output));
  assert(strstr(output,"(require-all (require-not (subpath \"/private/custody\")) (literal \"/private/custody/nested/keychain\"))"));
  assert(strstr(output,"(require-all (require-not (subpath \"/private/custody\")) (subpath \"/private\")) (with report)"));
  assert(strstr(output,"(allow file-read-data (require-not (subpath \"/private/custody\")))"));
  assert(strstr(output,"(require-all (require-not (subpath \"/private/custody\")) (extension \"com.apple.app-sandbox.read-write\"))"));
  assert(!strstr(output,"(allow file-issue-extension"));
  assert(strstr(output,"(global-name \"(allow file-read*)\")"));free(output);
  const char* malformed[]={"(allow file-read*", "(allow file-read* network-outbound)", "(allow file* file-issue-extension)",
    "(allow file-read-data literal)", "(allow file-read* (literal \"unterminated))", "(allow file-read-data))"};
  for(size_t i=0;i<sizeof(malformed)/sizeof(malformed[0]);++i)
    assert(!colossus_owned_custody_policy(malformed[i],strlen(malformed[i]),"/private/custody",&output)&&!output);
  const char* roots[]={"/", "/private/custody/", "/private//custody", "/private/../custody", "/private/./custody", "/private/\"custody", "/private/\\custody", "relative"};
  for(size_t i=0;i<sizeof(roots)/sizeof(roots[0]);++i)
    assert(!colossus_owned_custody_policy(source,strlen(source),roots[i],&output)&&!output);
  char huge[131073];memset(huge,' ',sizeof(huge));
  assert(!colossus_owned_custody_policy(huge,sizeof(huge),"/private/custody",&output)&&!output);
  assert(!colossus_owned_custody_policy("(version 1)\0x",13,"/private/custody",&output)&&!output);
}
