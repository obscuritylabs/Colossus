#include "image_verify.h"
#include <CommonCrypto/CommonDigest.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#define BODY_LIMIT (256U*1024U*1024U)
int colossus_owned_library_verify(const char* path,const uint8_t expected[32]) {
  if(!path||path[0]!='/'||!expected||getuid()==0||getuid()!=geteuid())return 0;
  int nonzero=0;for(size_t i=0;i<32;++i)nonzero|=expected[i]!=0;if(!nonzero)return 0;
  char canonical[PATH_MAX];if(!realpath(path,canonical)||strcmp(path,canonical))return 0;
  int fd=open(path,O_RDONLY|O_NOFOLLOW|O_CLOEXEC);if(fd<0)return 0;
  struct stat before,after;int valid=fstat(fd,&before)==0&&S_ISREG(before.st_mode)&&before.st_uid==geteuid()&&before.st_nlink==1&&(before.st_mode&0022)==0&&before.st_size>0&&before.st_size<=BODY_LIMIT;
  CC_SHA256_CTX hash;uint8_t digest[32];uint8_t bytes[32768];
  if(!CC_SHA256_Init(&hash))valid=0;
  off_t total=0;
  while(valid) {
    ssize_t count=read(fd,bytes,sizeof(bytes));if(count<0&&errno==EINTR)continue;
    if(count<0){valid=0;break;}if(count==0)break;
    total+=count;if(total>before.st_size||!CC_SHA256_Update(&hash,bytes,(CC_LONG)count))valid=0;
  }
  valid=valid&&total==before.st_size&&CC_SHA256_Final(digest,&hash)&&!memcmp(digest,expected,32)&&fstat(fd,&after)==0&&before.st_dev==after.st_dev&&before.st_ino==after.st_ino&&before.st_size==after.st_size&&before.st_mtimespec.tv_sec==after.st_mtimespec.tv_sec&&before.st_mtimespec.tv_nsec==after.st_mtimespec.tv_nsec&&before.st_ctimespec.tv_sec==after.st_ctimespec.tv_sec&&before.st_ctimespec.tv_nsec==after.st_ctimespec.tv_nsec&&before.st_mode==after.st_mode&&before.st_uid==after.st_uid&&before.st_nlink==after.st_nlink;
  if(close(fd)!=0)valid=0;memset(bytes,0,sizeof(bytes));memset(&hash,0,sizeof(hash));return valid;
}
