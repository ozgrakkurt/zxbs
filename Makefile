.PHONY: fuzz

# Run the fuzzer.
#
# Leak checking is disabled because the build system intentionally leaks
# memory (interned strings, the config, the library path lists, ... are all
# leaked on purpose and live for the whole duration of the process) so the
# leak checker would just report those intentional leaks and abort.
#
# Extra libFuzzer options can be passed with FUZZ_ARGS, e.g.:
#   make fuzz FUZZ_ARGS="-runs=100000 -max_total_time=60"
FUZZ_ARGS ?=

fuzz:
	ASAN_OPTIONS=detect_leaks=0:detect_odr_violation=0 cargo +nightly fuzz run fuzz_build -- -detect_leaks=0 $(FUZZ_ARGS)
