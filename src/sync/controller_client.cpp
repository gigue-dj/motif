#include "sync/controller_client.h"

#include <deque>
#include <mutex>

namespace kuzu {
namespace motif {
namespace sync {

namespace {

class InMemoryControllerClient final : public ControllerClient {
public:
    void applyMutation(const Mutation& m) override {
        std::lock_guard<std::mutex> lock(mu_);
        queue_.push_back(m);
    }

    void flush() override {
        // No-op in v0.0.1: there is no upstream to flush to. Real transport
        // lands in v0.0.2.
    }

private:
    std::mutex mu_;
    std::deque<Mutation> queue_;
};

} // namespace

std::unique_ptr<ControllerClient> makeInMemoryControllerClient() {
    return std::make_unique<InMemoryControllerClient>();
}

} // namespace sync
} // namespace motif
} // namespace kuzu
