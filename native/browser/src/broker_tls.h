#ifndef COLOSSUS_BROWSER_BROKER_TLS_H
#define COLOSSUS_BROWSER_BROKER_TLS_H

#include <algorithm>
#include <chrono>
#include <cstddef>
#include <cstdint>
#include <string>
#include <vector>

namespace colossus {
// Exact private downstream leaf trust; the broker separately verifies upstream
// CA, hostname and pinned address. This cannot add global or personal CA trust.
class BrokerTlsTrust {
 public:
  bool Configure(std::string origin, const uint8_t* leaf, size_t bytes,
                 uint32_t lifetime_ms, std::chrono::steady_clock::time_point now) {
    if (configured_ || origin.empty() || origin.size() > 4096 || !leaf ||
        !bytes || bytes > 65536 || !lifetime_ms || lifetime_ms > 300000)
      return false;
    origin_ = std::move(origin);
    leaf_.assign(leaf, leaf + bytes);
    deadline_ = now + std::chrono::milliseconds(lifetime_ms);
    configured_ = true;
    return true;
  }
  bool Matches(const std::string& origin, const uint8_t* leaf, size_t bytes,
               std::chrono::steady_clock::time_point now) const {
    return configured_ && now < deadline_ && origin == origin_ && leaf &&
      bytes == leaf_.size() && std::equal(leaf_.begin(), leaf_.end(), leaf);
  }
  void Revoke() { deadline_ = {}; leaf_.clear(); origin_.clear(); }
 private:
  bool configured_ = false;
  std::string origin_;
  std::vector<uint8_t> leaf_;
  std::chrono::steady_clock::time_point deadline_{};
};
}
#endif
