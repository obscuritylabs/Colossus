/* Disposable file-identity checks only; no dlopen, CEF, Mach or Keychain. */
#include "../image_verify.h"
#include <CommonCrypto/CommonDigest.h>
#include <assert.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>
int main(void) {
  char directory[]="/private/tmp/colossus-owned-image-XXXXXX";assert(mkdtemp(directory));
  char body[PATH_MAX],alias[PATH_MAX];assert(snprintf(body,sizeof(body),"%s/fixed Body.dylib",directory)>0);assert(snprintf(alias,sizeof(alias),"%s/alias",directory)>0);
  const char data[]="fixed library fixture";uint8_t expected[32],wrong[32];
  assert(CC_SHA256(data,sizeof(data)-1,expected));memcpy(wrong,expected,32);wrong[0]^=1;
  int fd=open(body,O_CREAT|O_EXCL|O_WRONLY,0500);assert(fd>=0);assert(write(fd,data,sizeof(data)-1)==sizeof(data)-1);assert(close(fd)==0);
  assert(colossus_owned_library_verify(body,expected));assert(!colossus_owned_library_verify(body,wrong));
  assert(link(body,alias)==0);assert(!colossus_owned_library_verify(body,expected));assert(unlink(alias)==0);
  assert(symlink(body,alias)==0);assert(!colossus_owned_library_verify(alias,expected));assert(unlink(alias)==0);
  assert(chmod(body,0520)==0);assert(!colossus_owned_library_verify(body,expected));assert(chmod(body,0500)==0);
  assert(colossus_owned_library_verify(body,expected));assert(unlink(body)==0);assert(rmdir(directory)==0);
}
