// Closed fixture-only capability transfer: one READ-ONLY plain receipt pipe.
// No Mach/task/audit port, control endpoint, observer or kqueue is transferred.
#include <errno.h>
#include <fcntl.h>
#include <libproc.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <unistd.h>

int colossus_fixture_parent_matches(unsigned child,unsigned parent) {
  struct proc_bsdinfo info;
  int size=proc_pidinfo((int)child,PROC_PIDTBSDINFO,0,&info,sizeof(info));
  return size==(int)sizeof(info)&&info.pbi_pid==child&&info.pbi_ppid==parent;
}
int colossus_fixture_send_receipt(int channel,int receipt) {
  struct stat state;
  if(fstat(receipt,&state)||!S_ISFIFO(state.st_mode)||(fcntl(receipt,F_GETFL)&O_ACCMODE)!=O_RDONLY)return -1;
  unsigned char byte='R';struct iovec data={&byte,1};
  union {struct cmsghdr alignment;unsigned char bytes[CMSG_SPACE(sizeof(int))];} control;
  memset(&control,0,sizeof(control));struct msghdr message={0};
  message.msg_iov=&data;message.msg_iovlen=1;message.msg_control=control.bytes;message.msg_controllen=sizeof(control.bytes);
  struct cmsghdr* header=CMSG_FIRSTHDR(&message);header->cmsg_level=SOL_SOCKET;header->cmsg_type=SCM_RIGHTS;header->cmsg_len=CMSG_LEN(sizeof(int));
  memcpy(CMSG_DATA(header),&receipt,sizeof(receipt));
  ssize_t count;do {count=sendmsg(channel,&message,0);}while(count<0&&errno==EINTR);
  return count==1?0:-1;
}
int colossus_fixture_receive_receipt(int channel) {
  unsigned char byte=0;struct iovec data={&byte,1};
  union {struct cmsghdr alignment;unsigned char bytes[CMSG_SPACE(sizeof(int))];} control;
  memset(&control,0,sizeof(control));struct msghdr message={0};
  message.msg_iov=&data;message.msg_iovlen=1;message.msg_control=control.bytes;message.msg_controllen=sizeof(control.bytes);
  ssize_t count;do {count=recvmsg(channel,&message,0);}while(count<0&&errno==EINTR);
  int descriptors[4];size_t descriptor_count=0,header_count=0;
  for(struct cmsghdr* header=CMSG_FIRSTHDR(&message);header;header=CMSG_NXTHDR(&message,header)) {
    ++header_count;
    if(header->cmsg_level==SOL_SOCKET&&header->cmsg_type==SCM_RIGHTS&&header->cmsg_len>=CMSG_LEN(0)) {
      size_t size=header->cmsg_len-CMSG_LEN(0);
      for(size_t offset=0;offset+sizeof(int)<=size&&descriptor_count<4;offset+=sizeof(int))
        memcpy(&descriptors[descriptor_count++],CMSG_DATA(header)+offset,sizeof(int));
    }
  }
  int received=descriptor_count==1?descriptors[0]:-1;
  struct stat state;
  if(count!=1||byte!='R'||message.msg_flags&(MSG_TRUNC|MSG_CTRUNC)||header_count!=1||descriptor_count!=1||received<0||fstat(received,&state)||!S_ISFIFO(state.st_mode)||(fcntl(received,F_GETFL)&O_ACCMODE)!=O_RDONLY||fcntl(received,F_SETFD,FD_CLOEXEC)) {for(size_t i=0;i<descriptor_count;++i)if(descriptors[i]>=0)close(descriptors[i]);return -1;}
  return received;
}
