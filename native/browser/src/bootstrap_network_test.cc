#include "bootstrap_network.h"
#include <cassert>

int main() {
  using colossus::BootstrapLiteralAddress;
  using colossus::BootstrapNumericHttpsOrigin;
  for (const auto* address : {"127.0.0.1", "::1", "2001:db8::1"})
    assert(BootstrapLiteralAddress(address));
  for (const auto* address : {"localhost", "127.1", "2130706433", "0177.0.0.1",
                            "127.0.0.256", "127.0.0.1:80", "[::1]", "fe80::1%en0", ""})
    assert(!BootstrapLiteralAddress(address));
  assert(!BootstrapLiteralAddress(std::string("127.0.0.1\0foreign", 17)));
  for (const auto* origin : {"https://127.0.0.1:19431", "https://[::1]:19431",
                            "https://[2001:db8::1]:65535"})
    assert(BootstrapNumericHttpsOrigin(origin));
  for (const auto* origin : {"http://127.0.0.1:19431", "HTTPS://127.0.0.1:19431",
                            "https://localhost:19431", "https://127.1:19431",
                            "https://127.0.0.1", "https://127.0.0.1:443",
                            "https://127.0.0.1:0", "https://127.0.0.1:019431",
                            "https://127.0.0.1:65536", "https://127.0.0.1:+19431",
                            "https://127.0.0.1:19431/", "https://127.0.0.1:19431?x=1",
                            "https://user@127.0.0.1:19431", "https://[::1%en0]:19431",
                            "https://::1:19431", "https://[2001:0db8::1]:19431"})
    assert(!BootstrapNumericHttpsOrigin(origin));
  assert(!BootstrapNumericHttpsOrigin(std::string("https://127.0.0.1:19431\0foreign", 29)));
}
