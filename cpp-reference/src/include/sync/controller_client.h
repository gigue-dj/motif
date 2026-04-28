#pragma once

#include <memory>

#include "sync/mutation.h"

namespace kuzu {
namespace motif {
namespace sync {

// ControllerClient is the contract between Motif and the upstream
// controller (SurrealDB in v0.0.1, custom Nebula later). It is the only
// piece of Motif that knows about the existence of an authoritative server.
//
// v0.0.1 ships with InMemoryControllerClient (no network). v0.0.2 swaps in
// a SurrealDB transport implementation behind this same interface.
//
// Thread-safety: implementations must be safe to call from the WAL commit
// path. The default in-memory implementation takes a mutex.
class ControllerClient {
public:
    virtual ~ControllerClient() = default;

    // Tee a successfully committed local mutation toward the controller.
    // Returns immediately. Implementations must not block on network I/O on
    // the commit path — queue and return.
    virtual void applyMutation(const Mutation& m) = 0;

    // Best-effort flush of any queued mutations. v0.0.1 is a no-op for the
    // in-memory client; v0.0.2 actually pushes to SurrealDB.
    virtual void flush() = 0;
};

// Default v0.0.1 implementation: a thread-safe in-memory queue. Used as the
// destination for the WAL commit hook so we can validate the architecture
// end-to-end without any network code.
std::unique_ptr<ControllerClient> makeInMemoryControllerClient();

} // namespace sync
} // namespace motif
} // namespace kuzu
