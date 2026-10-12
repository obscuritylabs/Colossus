#include "broker_tls.h"
#include <cassert>

int main() {
  using namespace std::chrono;
  colossus::BrokerTlsTrust trust;
  const uint8_t leaf[] = {1, 2, 3};
  const auto now = steady_clock::now();
  assert(!trust.Matches("https://owned.test", leaf, sizeof leaf, now));
  assert(!trust.Configure("https://owned.test", leaf, sizeof leaf, 300001, now));
  assert(trust.Configure("https://owned.test", leaf, sizeof leaf, 1000, now));
  assert(trust.Matches("https://owned.test", leaf, sizeof leaf, now));
  assert(!trust.Matches("https://foreign.test", leaf, sizeof leaf, now));
  assert(!trust.Matches("https://owned.test", leaf, 2, now));
  const uint8_t different[] = {1, 2, 4};
  assert(!trust.Matches("https://owned.test", different, sizeof different, now));
  assert(!trust.Matches("https://owned.test", leaf, sizeof leaf, now + milliseconds(1000)));
  assert(!trust.Configure("https://owned.test", leaf, sizeof leaf, 1000, now));
  trust.Revoke();
  assert(!trust.Matches("https://owned.test", leaf, sizeof leaf, now));
  assert(!trust.Configure("https://owned.test", leaf, sizeof leaf, 1000, now));
}
