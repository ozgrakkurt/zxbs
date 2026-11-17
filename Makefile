.PHONY: fuzz

# Seconds to fuzz each target for (override with `make fuzz FUZZ_TIME=120`).
FUZZ_TIME ?= 30

fuzz:
	cargo +nightly fuzz list | parallel -j0 ASAN_OPTIONS=detect_leaks=0:detect_odr_violation=0 cargo +nightly fuzz run {} -- -max_total_time=$(FUZZ_TIME)
