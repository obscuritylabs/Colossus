#ifndef COLOSSUS_BROWSER_BOOTSTRAP_NETWORK_H
#define COLOSSUS_BROWSER_BOOTSTRAP_NETWORK_H

// Pre-initialization validation must not construct GURL or call a CEF parser:
// Chromium adds schemes during CefInitialize and rejects any earlier registry
// read in DCHECK builds. These checks use only native numeric address parsing.
#if defined(_WIN32)
#include <winsock2.h>
#include <ws2tcpip.h>
#else
#include <arpa/inet.h>
#endif
#include <charconv>
#include <cstring>
#include <string>
#include <string_view>

namespace colossus {
inline bool BootstrapLiteralAddress(const std::string& address) {
  if (address.empty() || address.size() > 64 || address.find('\0') != std::string::npos)
    return false;
  in_addr ipv4{};
  in6_addr ipv6{};
  char canonical[INET6_ADDRSTRLEN]{};
  return (inet_pton(AF_INET, address.c_str(), &ipv4) == 1 &&
          inet_ntop(AF_INET, &ipv4, canonical, sizeof(canonical)) && address == canonical) ||
         (inet_pton(AF_INET6, address.c_str(), &ipv6) == 1 &&
          inet_ntop(AF_INET6, &ipv6, canonical, sizeof(canonical)) && address == canonical);
}

inline bool BootstrapNumericHttpsOrigin(std::string_view origin) {
  constexpr std::string_view prefix = "https://";
  if (!origin.starts_with(prefix) || origin.size() > 4096) return false;
  origin.remove_prefix(prefix.size());
  const auto separator = origin.rfind(':');
  if (separator == std::string_view::npos) return false;
  const auto authority = origin.substr(0, separator);
  const auto port_text = origin.substr(separator + 1);
  if (port_text.empty() || port_text.size() > 5 || port_text.front() == '0') return false;
  unsigned port = 0;
  const auto parsed = std::from_chars(port_text.data(), port_text.data() + port_text.size(), port);
  if (parsed.ec != std::errc{} || parsed.ptr != port_text.data() + port_text.size() ||
      !port || port > 65535 || port == 443) return false;
  const bool ipv6 = authority.size() > 2 && authority.front() == '[' && authority.back() == ']';
  const std::string host(ipv6 ? authority.substr(1, authority.size() - 2) : authority);
  if (host.empty() || host.size() > 64 || host.find('\0') != std::string::npos) return false;
  char canonical[INET6_ADDRSTRLEN]{};
  if (ipv6) {
    in6_addr value{};
    return inet_pton(AF_INET6, host.c_str(), &value) == 1 &&
           inet_ntop(AF_INET6, &value, canonical, sizeof(canonical)) && host == canonical;
  }
  in_addr value{};
  return inet_pton(AF_INET, host.c_str(), &value) == 1 &&
         inet_ntop(AF_INET, &value, canonical, sizeof(canonical)) && host == canonical;
}
}
#endif
