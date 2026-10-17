/* Pure source transformation regression: no Seatbelt application or CEF load. */
#include "../policy_transition.h"
#include <assert.h>
#include <stdlib.h>
#include <string.h>
int main(void) {
  const char* source="(version 1)\n(allow default)\n(allow process-fork)\n"
    "(allow process-exec (literal \"/sealed/front\") (with no-sandbox))\n"
    "(if (param \"x\") (allow process-exec (literal \"/sealed/body\")) #f)\n"
    "(allow file-read* (literal \"/sealed/contains-(allow process-fork)\"))\n"
    "(allow job-creation (literal \"/unregistered/helper\"))\n"
    "(allow mach-register (global-name \"unregistered.service\"))\n";
  const char* bindings="(let* ((directory (subpath \"/private/custody\"))) (allow process-fork) (allow file-read* directory))";
  char* bound=NULL;assert(colossus_owned_process_policy(bindings,strlen(bindings),&bound));
  assert(strstr(bound,"((directory (subpath \"/private/custody\"))) #f"));free(bound);
  char* output=NULL;assert(colossus_owned_process_policy(source,strlen(source),&output));
  assert(!strstr(output,"(allow process-exec"));
  assert(strstr(output,"(if (param \"x\") #f #f)"));
  assert(strstr(output,"contains-(allow process-fork)"));
  assert(!strstr(output,"(allow job-creation"));assert(!strstr(output,"(allow mach-register"));
  assert(strstr(output,"(deny job-creation mach-register)"));
  assert(strstr(output,"(deny process-fork process-exec*)"));assert(strstr(output,"(allow signal (target self))"));free(output);
  const char* malformed[]={"(allow process-exec", "(allow file-read* process-exec (literal \"/x\"))", "(allow process-exec (literal \"unterminated))", "(allow process-exec))", "(if () (allow process-exec))"};
  for(size_t i=0;i<sizeof(malformed)/sizeof(malformed[0]);++i)assert(!colossus_owned_process_policy(malformed[i],strlen(malformed[i]),&output)&&!output);
}
