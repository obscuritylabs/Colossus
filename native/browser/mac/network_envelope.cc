/* Deliberately valid C as well as C++. The minimal front compiles with -x c,
 * so C++ and system-framework initializers cannot acquire ambient resources. */
#include "network_envelope.h"
#include "network_policies.generated.h"
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <limits.h>

#define POLICY_CAPACITY (128U * 1024U)
struct builder { char* bytes; size_t length; int valid; unsigned removed_network_grants; };
static void append(struct builder* out, const char* format, ...) {
  if (!out->valid) return;
  va_list args;
  va_start(args, format);
  int count = vsnprintf(out->bytes + out->length, POLICY_CAPACITY - out->length, format, args);
  va_end(args);
  if (count < 0 || (size_t)count >= POLICY_CAPACITY - out->length) out->valid = 0;
  else out->length += (size_t)count;
}
static int path(const char* text) {
  if (!text || text[0] != '/' || strlen(text) < 2 || strlen(text) > 4096) return 0;
  if (strstr(text, "//") || strstr(text, "/./") || strstr(text, "/../")) return 0;
  size_t n = strlen(text);
  if (text[n-1] == '/' || strcmp(text+n-2, "/.") == 0 ||
      (n >= 3 && strcmp(text+n-3, "/..") == 0)) return 0;
  for (size_t i = 0; i < n; ++i) {
    unsigned char c = (unsigned char)text[i];
    if (c < 32 || c > 126 || c == '"' || c == '\\') return 0;
  }
  return 1;
}
static int inside(const char* child, const char* root) {
  size_t n = strlen(root);
  return strncmp(child, root, n) == 0 && (child[n] == '/' || child[n] == 0);
}
static int valid(const struct colossus_mac_network_binding* b) {
  if (!b || !b->audit_session || b->audit_session==UINT32_MAX || !b->proxy_port || !path(b->allocation_root) ||
      !path(b->bundle_path) || !path(b->profile_root) || !path(b->broker_root) ||
      !path(b->personal_home) || !b->bundle_id || !*b->bundle_id ||
      strlen(b->bundle_id) > 160) return 0;
  for (const char* c = b->bundle_id; *c; ++c)
    if (!((*c >= 'a' && *c <= 'z') || (*c >= 'A' && *c <= 'Z') ||
          (*c >= '0' && *c <= '9') || *c == '.' || *c == '-')) return 0;
  return inside(b->bundle_path, b->allocation_root) && inside(b->profile_root, b->allocation_root) &&
      strcmp(b->bundle_path,b->allocation_root) != 0 &&
      strcmp(b->profile_root,b->allocation_root) != 0 &&
      !inside(b->bundle_path,b->profile_root) && !inside(b->profile_root,b->bundle_path) &&
      !inside(b->broker_root,b->allocation_root) && !inside(b->allocation_root,b->broker_root);
}
static const char* suffix(enum colossus_mac_network_role r, int hash) {
#define ROLE(value, name) case value: return hash ? colossus_policy_##name##_sha256 : colossus_policy_##name
  switch (r) {
    ROLE(COLOSSUS_MAC_ROLE_RENDERER, renderer); ROLE(COLOSSUS_MAC_ROLE_GPU, gpu);
    ROLE(COLOSSUS_MAC_ROLE_NETWORK, network); ROLE(COLOSSUS_MAC_ROLE_UTILITY, utility);
    ROLE(COLOSSUS_MAC_ROLE_AUDIO, audio); ROLE(COLOSSUS_MAC_ROLE_CDM, cdm);
    ROLE(COLOSSUS_MAC_ROLE_MIRRORING, mirroring); ROLE(COLOSSUS_MAC_ROLE_PRINT_BACKEND, print_backend);
    ROLE(COLOSSUS_MAC_ROLE_PRINT_COMPOSITOR, print_compositor); ROLE(COLOSSUS_MAC_ROLE_PROXY_RESOLVER, proxy_resolver);
    ROLE(COLOSSUS_MAC_ROLE_SCREEN_AI, screen_ai); ROLE(COLOSSUS_MAC_ROLE_SPEECH_RECOGNITION, speech_recognition);
    ROLE(COLOSSUS_MAC_ROLE_ON_DEVICE_MODEL, on_device_model_execution);
    ROLE(COLOSSUS_MAC_ROLE_ON_DEVICE_TRANSLATION, on_device_translation);
    ROLE(COLOSSUS_MAC_ROLE_WEBNN, webnn_model_compilation);
  }
#undef ROLE
  return NULL;
}
const char* colossus_mac_role_policy_sha256(enum colossus_mac_network_role r) { return suffix(r,1); }
static int front_kind(const char* executable,const struct colossus_mac_network_binding* b) {
  static const char* suffixes[]={""," (Renderer)"," (GPU)"," (Plugin)"," (Alerts)"};
  char expected[PATH_MAX];
  for(int i=0;i<5;++i) {
    int n=snprintf(expected,sizeof(expected),"%s/Contents/Frameworks/Colossus Browser Helper%s.app/Contents/MacOS/Colossus Browser Helper%s",
      b->bundle_path,suffixes[i],suffixes[i]);
    if(n>0&&(size_t)n<sizeof(expected)&&strcmp(executable,expected)==0)return i;
  }
  return -1;
}
static void constraint(struct builder* out, const struct colossus_mac_network_binding* b, int proxy) {
  append(out,"\n; Colossus mandatory native envelope, appended after all stock role rules.\n"
    "(deny network*)\n(deny system-socket)\n(deny file-issue-extension)\n");
  if (proxy) append(out,"(allow network-outbound (remote tcp \"localhost:%u\"))\n",(unsigned)b->proxy_port);
  append(out,"(deny mach-lookup)\n"
    "(allow mach-lookup (global-name \"com.apple.logd\") (global-name \"com.apple.system.logger\")"
    " (global-name \"com.apple.system.opendirectoryd.libinfo\"))\n"
    "(deny file-read* file-write* (subpath \"%s\"))\n"
    "(deny file-read* (require-all (subpath \"%s\") (require-not (subpath \"%s\"))))\n"
    "(deny file-write* (require-not (subpath \"%s\")))\n"
    "(deny process-fork)\n(deny process-exec*)\n",
    b->broker_root,b->personal_home,b->allocation_root,b->profile_root);
}
static int finish(struct builder* out, char** source) {
  if (!out->valid) { free(out->bytes); return 0; }
  *source = out->bytes; return 1;
}
static void parameter(struct builder* out,const char* key,const char* value) {
  append(out,"(if (string=? key \"%s\") \"%s\"\n",key,value);
}
static int broad_network_grant(const char* source) {
  static const char* const operations[]={
    "network-outbound", "network-inbound", "network-bind", "system-socket"};
  if(strncmp(source,"(allow ",7)!=0)return 0;
  source+=7;
  for(size_t i=0;i<sizeof(operations)/sizeof(operations[0]);++i) {
    size_t length=strlen(operations[i]);
    if(strncmp(source,operations[i],length)==0 &&
       (source[length]==' ' || source[length]=='\t' ||
        source[length]=='\n' || source[length]==')'))return 1;
  }
  return 0;
}
static size_t expression_end(const char* source,size_t begin) {
  unsigned depth=0;
  int quoted=0,comment=0,escape=0;
  for(size_t i=begin;source[i];++i) {
    char value=source[i];
    if(comment){if(value=='\n')comment=0;continue;}
    if(quoted){
      if(escape)escape=0;
      else if(value=='\\')escape=1;
      else if(value=='"')quoted=0;
      continue;
    }
    if(value==';'){comment=1;continue;}
    if(value=='"'){quoted=1;continue;}
    if(value=='(')++depth;
    if(value==')' && --depth==0)return i+1;
  }
  return 0;
}
static void stock_source(struct builder* out,const char* text) {
  int quoted=0,comment=0,escape=0;
  for(size_t i=0;text[i];++i) {
    /* Stock network-service, proxy-resolver and print policies grant broad
     * TCP/UDP or system sockets. A later generic deny cannot override every
     * more-specific stock allow. Replace the complete pinned grant expression
     * with a false value, including when nested in an `if` body. The closed
     * proxy allowance is appended only after these grants are removed. */
    if(!quoted&&!comment&&broad_network_grant(text+i)) {
      size_t end=expression_end(text,i);
      if(!end){out->valid=0;return;}
      append(out,"#f"); ++out->removed_network_grants; i=end-1; continue;
    }
    /* Redirect only executable param calls. Quoted data, comments, regexes and
     * all policy rules retain their original bytes and meaning. Do not redefine
     * libsandbox's own param primitive or accept caller-supplied parameters. */
    if(!quoted&&!comment&&strncmp(text+i,"(param",6)==0 &&
      (text[i+6]==' '||text[i+6]=='\t'||text[i+6]=='\n')) {
      append(out,"(colossus-param"); i+=5; continue;
    }
    append(out,"%c",text[i]);
    if(comment){if(text[i]=='\n')comment=0;continue;}
    if(quoted){if(escape)escape=0;else if(text[i]=='\\')escape=1;else if(text[i]=='"')quoted=0;continue;}
    if(text[i]==';')comment=1;
    else if(text[i]=='"')quoted=1;
  }
}
static int helper(enum colossus_mac_network_role role,const struct colossus_mac_network_binding* b,
    const char* executable,const char* body,int32_t browser_pid,uint32_t os_version,char** source) {
  if (!source) return 0;
  *source=NULL;
  const char* stock=suffix(role,0);
  /* These roles require downloaded native components which this offline sealed
   * stage does not own. Never replace their required paths with browser input.
   * Preserve their pins for a future explicit component-enrollment contract. */
  if(role==COLOSSUS_MAC_ROLE_SCREEN_AI || role==COLOSSUS_MAC_ROLE_SPEECH_RECOGNITION ||
    role==COLOSSUS_MAC_ROLE_ON_DEVICE_TRANSLATION || role==COLOSSUS_MAC_ROLE_WEBNN)return 0;
  if (!valid(b) || !stock || !path(executable) || !inside(executable,b->bundle_path) ||
      (body && (!path(body) || !inside(body,b->bundle_path))) || browser_pid <= 1 ||
      os_version < 1400 || os_version > 9900) return 0;
  int kind=front_kind(executable,b);
  if(kind<0 || (role==COLOSSUS_MAC_ROLE_RENDERER && kind!=1) || (kind==1 && role!=COLOSSUS_MAC_ROLE_RENDERER) ||
    (role==COLOSSUS_MAC_ROLE_GPU && kind!=2) || (kind==2 && role!=COLOSSUS_MAC_ROLE_GPU) ||
    (kind==3 && role!=COLOSSUS_MAC_ROLE_CDM) || (kind==4 && role!=COLOSSUS_MAC_ROLE_UTILITY))return 0;
  if(body){size_t n=strlen(executable);if(strncmp(body,executable,n)||strcmp(body+n," Body"))return 0;}
  struct builder out={ (char*)malloc(POLICY_CAPACITY), 0, 1, 0 };
  if (!out.bytes) return 0;
  char pid[16],version[16],home[4120],cache[4120],temp[4120],log[4120];
  snprintf(pid,sizeof(pid),"%d",browser_pid); snprintf(version,sizeof(version),"%u",os_version);
  snprintf(home,sizeof(home),"%s/home",b->profile_root);
  snprintf(cache,sizeof(cache),"%s/cache",b->profile_root);
  snprintf(temp,sizeof(temp),"%s/tmp",b->profile_root);
  snprintf(log,sizeof(log),"%s/cef.log",b->profile_root);
  /* A closed local parameter function avoids accepting browser-provided strings.
   * Every inserted value has the same path/identifier grammar as the binding. */
  append(&out,"(version 1)\n(define (colossus-param key)\n");
  parameter(&out,"BROWSER_PID",pid); parameter(&out,"BUNDLE_ID",b->bundle_id);
  parameter(&out,"BUNDLE_PATH",b->bundle_path); parameter(&out,"EXECUTABLE_PATH",body?body:executable);
  parameter(&out,"USER_HOMEDIR_AS_LITERAL",home); parameter(&out,"OS_VERSION",version);
  parameter(&out,"LOG_FILE_PATH",log); parameter(&out,"DARWIN_USER_CACHE_DIR",cache);
  parameter(&out,"DARWIN_USER_DIR",temp); parameter(&out,"DARWIN_USER_TEMP_DIR",temp);
  parameter(&out,"DISABLE_SANDBOX_DENIAL_LOGGING","TRUE"); parameter(&out,"ENABLE_LOGGING","FALSE");
  parameter(&out,"NETWORK_USER_DIR_ACCESS","FALSE"); parameter(&out,"ODME_USER_DIR_ACCESS","FALSE");
  parameter(&out,"DISABLE_METAL_SHADER_CACHE","TRUE"); parameter(&out,"SYSTEM_PROXY_NETWORK_ACCESS","FALSE");
  parameter(&out,"NETWORK_SERVICE_STORAGE_PATHS_COUNT","1"); parameter(&out,"NETWORK_SERVICE_STORAGE_PATH_0",b->profile_root);
  append(&out,"#f"); for (int i=0;i<18;++i) append(&out,")"); append(&out,")\n");
  /* Remove only the redundant version declaration; all stock role rules remain. */
  const char* v=strstr(colossus_policy_common,"(version 1)");
  if (!v) { free(out.bytes); return 0; }
  append(&out,"%.*s",(int)(v-colossus_policy_common),colossus_policy_common);
  stock_source(&out,v+11);
  if(out.removed_network_grants!=1){free(out.bytes);return 0;}
  out.removed_network_grants=0;
  stock_source(&out,stock);
  unsigned expected=role==COLOSSUS_MAC_ROLE_NETWORK ? 3 :
    role==COLOSSUS_MAC_ROLE_PROXY_RESOLVER || role==COLOSSUS_MAC_ROLE_PRINT_BACKEND ? 1 : 0;
  if(out.removed_network_grants!=expected){free(out.bytes);return 0;}
  constraint(&out,b,role==COLOSSUS_MAC_ROLE_NETWORK);
  append(&out,"(allow mach-lookup (global-name \"%s.MachPortRendezvousServer.%d\"))\n",b->bundle_id,browser_pid);
  if (role==COLOSSUS_MAC_ROLE_RENDERER) append(&out,
    "(allow mach-lookup (global-name \"com.apple.FontObjectsServer\") (global-name \"com.apple.fonts\")"
    " (global-name \"com.apple.cvmsServ\") (global-name \"com.apple.lsd.mapdb\")"
    " (global-name \"com.apple.system.notification_center\"))\n");
  if (role==COLOSSUS_MAC_ROLE_GPU) append(&out,
    "(allow mach-lookup (global-name \"com.apple.CARenderServer\") (global-name \"com.apple.cvmsServ\")"
    " (global-name \"com.apple.gpumemd.source\") (global-name \"com.apple.windowserver.active\")"
    " (global-name \"com.apple.system.notification_center\")"
    " (xpc-service-name \"com.apple.MTLCompilerService\"))\n"
    "(allow file-issue-extension (require-all (extension-class \"com.apple.app-sandbox.read\")"
    " (subpath \"%s/tmp\")))\n",b->profile_root);
  if (body) append(&out,"(allow process-exec (literal \"%s\"))\n",body);
  return finish(&out,source);
}
int colossus_mac_helper_policy(enum colossus_mac_network_role r,const struct colossus_mac_network_binding* b,
    const char* e,int32_t p,uint32_t v,char** s) { return helper(r,b,e,NULL,p,v,s); }
int colossus_mac_helper_body_policy(enum colossus_mac_network_role r,const struct colossus_mac_network_binding* b,
    const char* e,const char* body,int32_t p,uint32_t v,char** s) { return helper(r,b,e,body,p,v,s); }
int colossus_mac_main_network_policy(const struct colossus_mac_network_binding* b,
    const char* const* fronts,size_t count,char** source) {
  if (!source) return 0;
  *source=NULL;
  if (!valid(b) || !fronts || count != 5) return 0;
  for(size_t i=0;i<count;++i) {
    if(!path(fronts[i]) || !inside(fronts[i],b->bundle_path) || front_kind(fronts[i],b)<0) return 0;
    for(size_t j=0;j<i;++j) if(strcmp(fronts[i],fronts[j])==0) return 0;
  }
  struct builder out={ (char*)malloc(POLICY_CAPACITY),0,1,0 };
  if(!out.bytes) return 0;
  append(&out,"(version 1)\n(allow default)\n"); constraint(&out,b,1);
  /* posix_spawn requires process-fork. Forked children retain the full main
   * envelope and may execute only the same exact sealed fronts. The external
   * audit-session owner must retain these descendants through cleanup. */
  append(&out,"(allow process-fork)\n");
  append(&out,"(allow mach-lookup (global-name \"com.apple.windowserver.active\")"
    " (global-name \"com.apple.CARenderServer\") (global-name \"com.apple.FontObjectsServer\")"
    " (global-name \"com.apple.fonts\") (global-name \"com.apple.lsd.mapdb\")"
    " (global-name \"com.apple.system.notification_center\"))\n");
  for(size_t i=0;i<count;++i) append(&out,"(allow process-exec (literal \"%s\") (with no-sandbox))\n",fronts[i]);
  return finish(&out,source);
}
static int number(const char* s,uint32_t maximum,uint32_t* result) {
  if (!*s || strlen(s)>10 || (*s=='0' && s[1])) return 0;
  uint64_t n=0; for(const char* c=s;*c;++c) { if(*c<'0'||*c>'9')return 0; n=n*10+(unsigned)(*c-'0'); if(n>maximum)return 0; }
  if(!n)return 0; *result=(uint32_t)n; return 1;
}
int colossus_mac_network_binding_parse(char* s,size_t size,struct colossus_mac_network_binding* b) {
  if(!s || !b || size==0 || size>32*1024 || memchr(s,0,size))return 0;
  char* fields[9]; size_t n=0; char* start=s;
  for(size_t i=0;i<size;++i) if(s[i]=='\n') { if(n==9)return 0; s[i]=0; fields[n++]=start; start=s+i+1; }
  if(n!=9 || start!=s+size || strcmp(fields[0],"COLOSSUS_MAC_NETWORK_ENVELOPE_V1"))return 0;
  uint32_t asid,port;
  if(!number(fields[1],UINT32_MAX,&asid)||!number(fields[2],65535,&port))return 0;
  *b=(struct colossus_mac_network_binding){asid,(uint16_t)port,fields[3],fields[4],fields[5],fields[6],fields[7],fields[8]};
  return valid(b);
}
