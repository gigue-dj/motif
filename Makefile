# Motif - helper frontend to cmake.
# Tip: to see the actual commands that will be run for any target, use `make -n <target>`.
#
# v0.0.1 supports only the wasm target. Native release/debug targets are kept
# for development convenience but are not the shipping artifact.

.DEFAULT_GOAL := wasm
.PHONY: \
	release relwithdebinfo debug \
	wasm wasmtest \
	test-build test \
	tidy clangd-diagnostics \
	install clean
.ONESHELL:
.SHELLFLAGS = -ec

BUILD_TYPE ?=
BUILD_PATH ?=
EXTRA_CMAKE_FLAGS ?=
CLANGD_DIAGNOSTIC_INSTANCES ?= 4
PREFIX ?= install
TEST_JOBS ?= 10

ifeq ($(shell uname -s 2>/dev/null),Linux)
	NUM_THREADS ?= $(shell expr $(shell nproc) \* 2 / 3)
else ifeq ($(shell uname -s 2>/dev/null),Darwin)
	NUM_THREADS ?= $(shell expr $(shell sysctl -n hw.ncpu) \* 2 / 3)
else
	NUM_THREADS ?= 1
endif
export CMAKE_BUILD_PARALLEL_LEVEL=$(NUM_THREADS)

ifeq ($(OS),Windows_NT)
	GEN ?= Ninja
	SHELL := cmd.exe
	.SHELLFLAGS := /c
endif

ifdef GEN
	CMAKE_FLAGS += -G "$(GEN)"
endif

ifdef ASAN
	CMAKE_FLAGS += -DENABLE_ADDRESS_SANITIZER=$(ASAN)
endif
ifdef TSAN
	CMAKE_FLAGS += -DENABLE_THREAD_SANITIZER=$(TSAN)
endif
ifdef UBSAN
	CMAKE_FLAGS += -DENABLE_UBSAN=$(UBSAN)
endif
ifdef RUNTIME_CHECKS
	CMAKE_FLAGS += -DENABLE_RUNTIME_CHECKS=$(RUNTIME_CHECKS)
endif
ifdef WERROR
	CMAKE_FLAGS += -DENABLE_WERROR=$(WERROR)
endif
ifdef LTO
	CMAKE_FLAGS += -DENABLE_LTO=$(LTO)
endif
ifdef SKIP_SINGLE_FILE_HEADER
	CMAKE_FLAGS += -DBUILD_SINGLE_FILE_HEADER=FALSE
endif
ifdef SINGLE_THREADED
	CMAKE_FLAGS += -DSINGLE_THREADED=$(SINGLE_THREADED)
endif
ifdef WASM_NODEFS
	CMAKE_FLAGS += -DWASM_NODEFS=$(WASM_NODEFS)
endif
ifdef EXTRA_CMAKE_FLAGS
	CMAKE_FLAGS += $(EXTRA_CMAKE_FLAGS)
endif

# Native development targets (not shipped — useful for local iteration on the
# core engine without the wasm/emscripten toolchain).
release:
	$(call run-cmake-release,)

relwithdebinfo:
	$(call run-cmake-relwithdebinfo,)

debug:
	$(call run-cmake-debug,)

test-build:
	$(call run-cmake-relwithdebinfo, -DBUILD_TESTS=TRUE)

test: test-build
	ctest --test-dir build/$(call get-build-path,RelWithDebInfo)/test --output-on-failure -j ${TEST_JOBS}

# Shipping artifact: wasm.
wasm:
	mkdir -p build/wasm && cd build/wasm &&\
	emcmake cmake $(CMAKE_FLAGS) -DCMAKE_BUILD_TYPE=$(call get-build-type,Release) -DBUILD_WASM=TRUE -DBUILD_TESTS=FALSE -DBUILD_SHELL=FALSE  ../.. && \
	cmake --build . --config $(call get-build-type,Release) -j $(NUM_THREADS)

wasmtest:
	mkdir -p build/wasm && cd build/wasm &&\
	emcmake cmake $(CMAKE_FLAGS) -DCMAKE_BUILD_TYPE=$(call get-build-type,Release) -DBUILD_WASM=TRUE -DBUILD_TESTS=TRUE -DBUILD_SHELL=FALSE  ../.. && \
	cmake --build . --config $(call get-build-type,Release) -j $(NUM_THREADS) &&\
	cd ../.. && ctest --test-dir  build/wasm/test/ --output-on-failure -j ${TEST_JOBS} --timeout 600

tidy:
	$(call config-cmake-release,)
	run-clang-tidy -p build/$(call get-build-path,Release) -quiet -j $(NUM_THREADS) \
		"^$(realpath src)|$(realpath tools)"

clangd-diagnostics:
	$(call config-cmake-release,)
	find src -name *.h -or -name *.cpp | xargs \
		./scripts/get-clangd-diagnostics.py --compile-commands-dir build/$(call get-build-path,Release) \
		-j $(NUM_THREADS) --instances $(CLANGD_DIAGNOSTIC_INSTANCES)

install:
	cmake --install build/$(call get-build-path,Release) --prefix $(PREFIX)

clean:
	cmake -E rm -rf build


# Utils
lowercase = $(if $(filter Release,$(1)),release,$(if $(filter RelWithDebInfo,$(1)),relwithdebinfo,$(if $(filter Debug,$(1)),debug,$(1))))
get-build-type = $(if $(BUILD_TYPE),$(BUILD_TYPE),$1)
get-build-path = $(if $(BUILD_PATH),$(BUILD_PATH),$(call lowercase,$(call get-build-type,$(1))))

define config-cmake
	cmake -B build/$(call get-build-path,$1) -DCMAKE_BUILD_TYPE=$(call get-build-type,$1) $2 $(CMAKE_FLAGS) $(EXTRA_CMAKE_FLAGS) .
endef

define build-cmake
	cmake --build build/$(call get-build-path,$1) --config $(call get-build-type,$1)
endef

define run-cmake-debug
	$(call config-cmake,Debug,$2)
	$(call build-cmake,Debug)
endef

define config-cmake-release
	$(call config-cmake,Release,$1)
endef

define config-cmake-relwithdebinfo
	$(call config-cmake,RelWithDebInfo,$1)
endef

define run-cmake-release
	$(call config-cmake-release,$1)
	$(call build-cmake,Release)
endef

define run-cmake-relwithdebinfo
	$(call config-cmake-relwithdebinfo,$1)
	$(call build-cmake,RelWithDebInfo)
endef
