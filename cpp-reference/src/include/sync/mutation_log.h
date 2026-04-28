#pragma once

#include <atomic>
#include <deque>
#include <mutex>

#include "sync/mutation.h"

namespace kuzu {
namespace motif {
namespace sync {

// MutationLog is the bridge between the local WAL commit path and the
// ControllerClient. It assigns monotonic localSeq values to mutations and
// hands them off to the configured client.
//
// v0.0.1 keeps the log entirely in-process. v0.0.2 will persist the log
// alongside the WAL so queued mutations survive crashes and offline-mode
// restarts.
class MutationLog {
public:
    MutationLog() = default;

    // Records a mutation, assigns localSeq, and forwards to the client if
    // one is registered. If no client is registered the mutation is held in
    // the buffer (bounded retention is a v0.0.2 concern).
    void record(Mutation m);

    // Drain the buffered mutations (e.g. for tests).
    std::deque<Mutation> drain();

    // Wire a client. Subsequent record() calls will forward to it.
    // Pre-existing buffered mutations are NOT replayed; the caller is
    // responsible for that if desired.
    void setClient(class ControllerClient* client);

private:
    std::mutex mu_;
    std::atomic<uint64_t> nextSeq_{1};
    std::deque<Mutation> buffer_;
    class ControllerClient* client_{nullptr};
};

} // namespace sync
} // namespace motif
} // namespace kuzu
