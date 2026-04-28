#pragma once

#include <cstdint>
#include <string>
#include <vector>

namespace kuzu {
namespace motif {
namespace sync {

// Identity of the actor who produced a mutation. Per-user + per-device
// because v0.0.1 anticipates compromised / shared devices: the controller
// must be able to distinguish "this user from device A" from "same user
// from device B" for audit and conflict resolution.
struct ActorId {
    std::string userId;
    std::string deviceId;
};

enum class MutationKind : uint8_t {
    NodeInsert = 0,
    NodeUpdate = 1,
    NodeDelete = 2,
    RelInsert = 3,
    RelUpdate = 4,
    RelDelete = 5,
};

// A controller-bound mutation record. Populated from a successful local
// transaction commit. v0.0.1 keeps this opaque-ish — we serialize the WAL
// record bytes verbatim plus enough metadata for the controller to route.
// v0.0.2 will replace `walPayload` with a structured diff once the
// SurrealQL boundary is in place.
struct Mutation {
    uint64_t localSeq{0};
    MutationKind kind{MutationKind::NodeInsert};
    ActorId actor;
    std::string tableName;
    std::vector<uint8_t> walPayload;
};

} // namespace sync
} // namespace motif
} // namespace kuzu
