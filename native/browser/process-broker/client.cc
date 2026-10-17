// Standalone byte-stream client; no CEF dependency, Mach calls or FD transfers.
#include "client.h"
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <array>
#include <cstring>
#include <limits>
#include <map>
#include <mutex>
#include <thread>
#if __has_include("base/no_destructor.h")
#include "base/no_destructor.h"
#endif

namespace {
struct Child { uint64_t slot; bool resumed = false; bool exited = false; int status = 0; };
struct Client {
  std::mutex lock;
  int fd = -1;
  pid_t browser = 0;
  std::array<uint8_t,16> nonce{};
  uint64_t sequence = 0;
  bool broken = false;
  std::map<pid_t,Child> children;
};
Client& State() {
  // The channel and ledger live until process exit. Chromium prohibits exit-time
  // destructors; the independently retained creator owns native child cleanup.
#if __has_include("base/no_destructor.h")
  static base::NoDestructor<Client> client;
  return *client;
#else
  static Client* const client = new Client;
  return *client;
#endif
}
bool Broken(Client& client) {
  client.broken=true;
  // Shut down the shared socket even if trusted composition retains a watcher
  // clone. The real owner's byte-stream EOF must fence and clean its ledger.
  if(client.fd>=0)shutdown(client.fd,SHUT_RDWR);
  return false;
}
void Number(std::vector<uint8_t>& bytes, uint64_t number, size_t size) {
  for (size_t i=0;i<size;++i) bytes.push_back(static_cast<uint8_t>(number >> (8*(size-i-1))));
}
uint64_t Decode(const uint8_t* bytes,size_t size) {
  uint64_t value=0; for(size_t i=0;i<size;++i)value=(value<<8)|bytes[i]; return value;
}
bool Exact(int fd,void* input,size_t size,bool write_bytes) {
  auto* cursor=static_cast<uint8_t*>(input);
  while(size) {
    const ssize_t count=write_bytes?write(fd,cursor,size):read(fd,cursor,size);
    if(count<0&&errno==EINTR)continue;
    if(count<=0)return false;
    cursor+=count; size-=static_cast<size_t>(count);
  }
  return true;
}
bool Exchange(Client& client,const std::vector<uint8_t>& payload,
              uint64_t expected_slot,uint64_t* slot,pid_t* pid,bool* exited,int* status) {
  if(client.fd<0||client.broken||payload.empty()||payload.size()>65536-36||client.sequence==std::numeric_limits<uint64_t>::max())return false;
  std::vector<uint8_t> frame={'C','B','R','O','K','0','0','1'};
  Number(frame,++client.sequence,8); frame.insert(frame.end(),client.nonce.begin(),client.nonce.end()); Number(frame,payload.size(),4);
  frame.insert(frame.end(),payload.begin(),payload.end());
  std::array<uint8_t,36> header{};
  if(!Exact(client.fd,frame.data(),frame.size(),true)||!Exact(client.fd,header.data(),header.size(),false))return Broken(client);
  const uint64_t length=Decode(header.data()+32,4);
  if(std::memcmp(header.data(),"CBROK001",8)||Decode(header.data()+8,8)!=client.sequence||std::memcmp(header.data()+16,client.nonce.data(),16)||(length!=14&&length!=18))return Broken(client);
  std::array<uint8_t,18> response{};
  if(!Exact(client.fd,response.data(),static_cast<size_t>(length),false)||response[0]!=6||response[13]!=(length==18?1:0))return Broken(client);
  *slot=Decode(response.data()+1,8);
  const uint64_t raw_pid=Decode(response.data()+9,4);
  if(*slot==0||(expected_slot&&*slot!=expected_slot)||raw_pid>static_cast<uint64_t>(std::numeric_limits<pid_t>::max()))return Broken(client);
  *pid=static_cast<pid_t>(raw_pid); *exited=length==18;
  *status=*exited?static_cast<int>(Decode(response.data()+14,4)):0;
  return true;
}
uint8_t Role(const std::vector<std::string>& arguments) {
  std::string process,service;
  for(const auto& argument:arguments) {
    if(argument.starts_with("--type=")) { if(!process.empty())return 0; process=argument.substr(7); }
    if(argument.starts_with("--service-sandbox-type=")) { if(!service.empty())return 0; service=argument.substr(23); }
  }
  if(process=="renderer"&&service.empty())return 1;
  if(process=="gpu-process"&&service.empty())return 2;
  if(process=="utility"&&service=="network")return 3;
  if(process=="utility"&&service=="utility")return 4;
  return 0;
}
bool ChildOperation(Client& client,uint8_t operation,pid_t pid) {
  auto found=client.children.find(pid); if(found==client.children.end())return false;
  auto& child=found->second;
  if(operation==2&&(child.resumed||child.exited))return false;
  if(child.exited)return operation!=2;
  std::vector<uint8_t> payload{operation}; Number(payload,child.slot,8);
  uint64_t slot=0; pid_t reply_pid=0; bool exited=false; int status=0;
  if(!Exchange(client,payload,child.slot,&slot,&reply_pid,&exited,&status))return false;
  if(reply_pid!=pid)return Broken(client);
  child.exited=exited; child.status=status;
  if(operation==2)child.resumed=true;
  return true;
}
}  // namespace

extern "C" bool colossus_owned_process_install(int fd,const uint8_t nonce[16],pid_t browser) {
  auto& client=State(); std::lock_guard guard(client.lock);
  if(client.browser||fd<3||browser<=1||!nonce)return false;
  bool nonzero=false; for(size_t i=0;i<16;++i)nonzero|=nonce[i]!=0;
  timeval timeout{2,0};
  if(!nonzero||fcntl(fd,F_SETFD,FD_CLOEXEC)!=0||
     setsockopt(fd,SOL_SOCKET,SO_RCVTIMEO,&timeout,sizeof(timeout))!=0||
     setsockopt(fd,SOL_SOCKET,SO_SNDTIMEO,&timeout,sizeof(timeout))!=0)return false;
  client.fd=fd; client.browser=browser; std::memcpy(client.nonce.data(),nonce,16); return true;
}
extern "C" bool colossus_owned_helper_install(pid_t browser) {
  auto& client=State(); std::lock_guard guard(client.lock);
  if(client.browser||browser<=1)return false; client.browser=browser; return true;
}
namespace colossus_owned_process {
bool Enabled() { auto& c=State(); std::lock_guard guard(c.lock); return c.browser>0; }
pid_t LogicalBrowserPid() { auto& c=State(); std::lock_guard guard(c.lock); return c.browser>0?c.browser:getppid(); }
pid_t SpawnStopped(const std::vector<std::string>& arguments) {
  auto& client=State(); std::lock_guard guard(client.lock);
  const uint8_t role=Role(arguments); if(!role||arguments.empty()||arguments.size()>128||client.children.size()>=64)return 0;
  std::vector<uint8_t> payload{1,role}; Number(payload,arguments.size(),2);
  for(const auto& argument:arguments) { if(argument.empty()||argument.size()>4096)return 0; Number(payload,argument.size(),2); payload.insert(payload.end(),argument.begin(),argument.end()); }
  uint64_t slot=0; pid_t pid=0; bool exited=false; int status=0;
  if(!Exchange(client,payload,0,&slot,&pid,&exited,&status))return 0;
  // Chromium's base::Process stores only a PID, so an old handle cannot safely
  // be rebound to a new slot after PID reuse. End this allocation instead.
  if(pid<=1||exited||client.children.contains(pid)) { Broken(client);return 0; }
  client.children.emplace(pid,Child{slot}); return pid;
}
bool Resume(pid_t pid) { auto& client=State(); std::lock_guard guard(client.lock); return ChildOperation(client,2,pid); }
bool Status(pid_t pid,bool* exited,int* raw_status) {
  if(!exited||!raw_status)return false;
  auto& client=State(); std::lock_guard guard(client.lock);
  if(!ChildOperation(client,3,pid))return false;
  const auto& child=client.children.at(pid); *exited=child.exited; *raw_status=child.status; return true;
}
bool Terminate(pid_t pid) { auto& client=State(); std::lock_guard guard(client.lock); return ChildOperation(client,4,pid); }
bool WaitForExit(pid_t pid,std::chrono::milliseconds timeout,int* raw_wait_status) {
  if(!raw_wait_status||timeout.count()<0)return false;
  const bool forever=timeout==std::chrono::milliseconds::max();
  const auto started=std::chrono::steady_clock::now();
  for(;;) {
    bool exited=false;int status=0;
    if(!Status(pid,&exited,&status))return false;
    if(exited){*raw_wait_status=status;return true;}
    if(!forever&&std::chrono::steady_clock::now()-started>=timeout)return false;
    std::this_thread::sleep_for(std::chrono::milliseconds(10));
  }
}
}  // namespace colossus_owned_process
