// A byte-stream peer fixture only: no child processes, CEF, Mach, audit or launchd.
#include "../client.h"
#include <sys/socket.h>
#include <unistd.h>
#include <array>
#include <cassert>
#include <cstring>
#include <thread>
#include <vector>

namespace {
void Exact(int fd,void* data,size_t size,bool writing) {
  auto* cursor=static_cast<unsigned char*>(data);
  while(size) { const auto count=writing?write(fd,cursor,size):read(fd,cursor,size); assert(count>0); cursor+=count; size-=static_cast<size_t>(count); }
}
uint64_t Number(const unsigned char* data,size_t size) { uint64_t n=0;for(size_t i=0;i<size;++i)n=(n<<8)|data[i];return n; }
void Append(std::vector<unsigned char>& bytes,uint64_t n,size_t size) { for(size_t i=0;i<size;++i)bytes.push_back(static_cast<unsigned char>(n>>(8*(size-i-1)))); }
void Peer(int fd) {
  const std::array<unsigned char,5> operations{1,2,3,4,1};
  for(size_t i=0;i<operations.size();++i) {
    std::array<unsigned char,36> header{}; Exact(fd,header.data(),header.size(),false);
    assert(!std::memcmp(header.data(),"CBROK001",8)); assert(Number(header.data()+8,8)==i+1);
    const auto size=Number(header.data()+32,4); assert(size>0&&size<65536-36);
    std::vector<unsigned char> payload(size);Exact(fd,payload.data(),payload.size(),false);assert(payload[0]==operations[i]);
    if(i==0||i==4) { assert(payload[1]==1);assert(Number(payload.data()+2,2)==2); }
    else { assert(size==9);assert(Number(payload.data()+1,8)==88); }
    std::vector<unsigned char> reply{6};Append(reply,88,8);Append(reply,777,4);reply.push_back(i==3?1:0);if(i==3)Append(reply,9,4);
    header[32]=0;header[33]=0;header[34]=0;header[35]=static_cast<unsigned char>(reply.size());
    if(i==4) {
      header[16]^=1;Exact(fd,header.data(),header.size(),true);
      unsigned char byte=0;assert(read(fd,&byte,1)==0);continue;
    }
    Exact(fd,header.data(),header.size(),true);Exact(fd,reply.data(),reply.size(),true);
  }
  close(fd);
}
void WrongPidPeer(int fd,uint8_t operation) {
  std::vector<uint8_t> operations{1};
  if(operation!=2)operations.push_back(2);
  operations.push_back(operation);
  for(size_t index=0;index<operations.size();++index) {
    std::array<unsigned char,36> header{};Exact(fd,header.data(),header.size(),false);
    assert(!std::memcmp(header.data(),"CBROK001",8));
    const auto length=Number(header.data()+32,4);assert(length>0&&length<65536-36);
    std::vector<unsigned char> payload(length);Exact(fd,payload.data(),payload.size(),false);
    assert(payload[0]==operations[index]);
    std::vector<unsigned char> reply{6};Append(reply,88,8);
    Append(reply,index+1==operations.size()?778:777,4);reply.push_back(0);
    header[32]=0;header[33]=0;header[34]=0;header[35]=14;
    Exact(fd,header.data(),header.size(),true);Exact(fd,reply.data(),reply.size(),true);
  }
  // A correct slot with a different PID must fence the entire channel. A
  // later Spawn cannot send any bytes instead of treating this as local failure.
  unsigned char byte=0;assert(read(fd,&byte,1)==0);close(fd);
}
}  // namespace
int main(int argc,char** argv) {
  int sockets[2];assert(socketpair(AF_UNIX,SOCK_STREAM,0,sockets)==0);
  std::array<uint8_t,16> nonce{};nonce.fill(7);
  assert(colossus_owned_process_install(sockets[0],nonce.data(),4242));
  assert(!colossus_owned_process_install(sockets[0],nonce.data(),4242));
  assert(colossus_owned_process::Enabled());assert(colossus_owned_process::LogicalBrowserPid()==4242);
  if(argc==2) {
    uint8_t operation=0;
    if(std::strcmp(argv[1],"wrong-reply-pid-resume")==0)operation=2;
    if(std::strcmp(argv[1],"wrong-reply-pid-status")==0)operation=3;
    if(std::strcmp(argv[1],"wrong-reply-pid-terminate")==0)operation=4;
    assert(operation);
    std::thread peer(WrongPidPeer,sockets[1],operation);
    assert(colossus_owned_process::SpawnStopped({"--type=renderer","--shared-files="})==777);
    if(operation!=2)assert(colossus_owned_process::Resume(777));
    bool exited=true;int status=-1;
    if(operation==2)assert(!colossus_owned_process::Resume(777));
    if(operation==3)assert(!colossus_owned_process::Status(777,&exited,&status));
    if(operation==4)assert(!colossus_owned_process::Terminate(777));
    assert(colossus_owned_process::SpawnStopped({"--type=renderer","--shared-files="})==0);
    peer.join();close(sockets[0]);return 0;
  }
  assert(argc==1);
  std::thread peer(Peer,sockets[1]);
  assert(colossus_owned_process::SpawnStopped({"--type=bad"})==0);
  const auto pid=colossus_owned_process::SpawnStopped({"--type=renderer","--shared-files="});assert(pid==777);
  assert(colossus_owned_process::Resume(pid));assert(!colossus_owned_process::Resume(pid));
  bool exited=true;int status=-1;assert(colossus_owned_process::Status(pid,&exited,&status));assert(!exited&&status==0);
  assert(colossus_owned_process::Terminate(pid));assert(colossus_owned_process::Status(pid,&exited,&status));assert(exited&&status==9);
  assert(colossus_owned_process::WaitForExit(pid,std::chrono::milliseconds(0),&status));assert(status==9);
  assert(!colossus_owned_process::Terminate(778));
  // Wrong allocation reply must close the underlying channel, even while the
  // browser remains alive. That EOF gives the real broker its cleanup trigger.
  assert(colossus_owned_process::SpawnStopped({"--type=renderer","--shared-files="})==0);
  peer.join();close(sockets[0]);
}
