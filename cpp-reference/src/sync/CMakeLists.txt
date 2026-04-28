add_library(motif_sync
        OBJECT
        controller_client.cpp
        mutation_log.cpp)

set(ALL_OBJECT_FILES
        ${ALL_OBJECT_FILES} $<TARGET_OBJECTS:motif_sync>
        PARENT_SCOPE)
