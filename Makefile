# Seconds to fuzz each target for.
FUZZ_TIME ?= 30

# Number of fuzz targets to run concurrently (defaults to the CPU count).
FUZZ_JOBS ?= $(shell nproc 2>/dev/null || echo 1)

# Extra libFuzzer options passed to every target, e.g.:
#   make fuzz FUZZ_ARGS="-runs=100000"
FUZZ_ARGS ?=

# Ask cargo-fuzz which targets exist instead of globbing the directory.
FUZZ_TARGETS := $(shell cargo +nightly fuzz list 2>/dev/null)

.PHONY: fuzz fuzz-targets $(addprefix run-,$(FUZZ_TARGETS))

# Run every fuzz target in parallel, FUZZ_TIME seconds each.
fuzz:
	@$(MAKE) --no-print-directory -j$(FUZZ_JOBS) -Otarget $(addprefix run-,$(FUZZ_TARGETS))

# Run a single fuzz target, e.g. `make run-fuzz_wal`.
#
# Leak checking is disabled because the build system intentionally leaks
# memory (interned strings, the config, the library path lists, ... are all
# leaked on purpose and live for the whole duration of the process) so the
# leak checker would just report those intentional leaks and abort.
$(addprefix run-,$(FUZZ_TARGETS)): run-%:
	@echo "==> $* (${FUZZ_TIME}s)"
	ASAN_OPTIONS=detect_leaks=0:detect_odr_violation=0 cargo +nightly fuzz run $* -- -detect_leaks=0 -max_total_time=$(FUZZ_TIME) $(FUZZ_ARGS)

# List the available fuzz targets.
fuzz-targets:
	@for target in $(FUZZ_TARGETS); do echo $$target; done
