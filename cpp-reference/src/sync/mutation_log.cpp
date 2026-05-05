#include "sync/mutation_log.h"

#include "sync/controller_client.h"

namespace kuzu {
namespace motif {
namespace sync {

void MutationLog::record(Mutation m) {
    std::lock_guard<std::mutex> lock(mu_);
    m.localSeq = nextSeq_.fetch_add(1, std::memory_order_relaxed);
    if (client_ != nullptr) {
        client_->applyMutation(m);
    } else {
        buffer_.push_back(std::move(m));
    }
}

std::deque<Mutation> MutationLog::drain() {
    std::lock_guard<std::mutex> lock(mu_);
    std::deque<Mutation> out;
    out.swap(buffer_);
    return out;
}

void MutationLog::setClient(ControllerClient* client) {
    std::lock_guard<std::mutex> lock(mu_);
    client_ = client;
}

} // namespace sync
} // namespace motif
} // namespace kuzu
