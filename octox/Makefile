TARGET  := riscv64gc-unknown-none-elf
PYTHON  := python3
GRADE   := tests/grade_scheduler.py
TIMEOUT := 300
FULL_BENCH_TIMEOUT ?= 1200
BENCH_RESULTS ?= /tmp/octox_full_bench.json
BENCH_RELEASE_RESULTS ?= /tmp/octox_full_bench_release.json

# Set RELEASE=1 to build and test with --release (optimized).
#   make bench RELEASE=1
#   make grade-cfs RELEASE=1
RELEASE ?=
ifdef RELEASE
  PROFILE     := release
  CARGO_FLAGS := --release
  GRADE_FLAGS := --release
else
  PROFILE     := debug
  CARGO_FLAGS :=
  GRADE_FLAGS :=
endif

KERNEL  := target/$(TARGET)/$(PROFILE)/octox
FS_IMG  := target/fs.img

QEMU := qemu-system-riscv64
QEMUOPTS := -machine virt -bios none -m 524M -smp 4 -nographic \
	-serial mon:stdio -global virtio-mmio.force-legacy=false \
	-drive file=$(FS_IMG),if=none,format=raw,id=x0 \
	-device virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0

# ── Build ───────────────────────────────────────────────────────────

.PHONY: all build clean qemu test bench bench-release grade miri \
	bench-report bench-release-report \
	test-cfs test-rr test-mlfq test-o1 test-eevdf test-drl test-hybrid-drl \
	bench-cfs bench-rr bench-mlfq bench-o1 bench-eevdf bench-drl bench-hybrid-drl \
	grade-cfs grade-rr grade-mlfq grade-o1 grade-eevdf grade-drl grade-hybrid-drl

all: grade

build:
	cargo build --target $(TARGET) $(CARGO_FLAGS)

clean:
	rm -rf target

miri:
	cd tests/miri && cargo +nightly miri test

# ── Run in QEMU (interactive) ──────────────────────────────────────

qemu: build
	$(QEMU) $(QEMUOPTS) -kernel $(KERNEL)

# ── Full suite (all schedulers) ────────────────────────────────────

grade: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --scheduler all --timeout $(TIMEOUT) $(GRADE_FLAGS)

test: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode test --scheduler all --timeout $(TIMEOUT) $(GRADE_FLAGS)

bench: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode bench --scheduler all --timeout $(TIMEOUT) $(GRADE_FLAGS)

bench-release:
	$(MAKE) bench RELEASE=1

bench-report: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode bench --scheduler all --timeout $(FULL_BENCH_TIMEOUT) $(GRADE_FLAGS) --results $(BENCH_RESULTS)

bench-release-report:
	$(MAKE) bench-report RELEASE=1 BENCH_RESULTS=$(BENCH_RELEASE_RESULTS)

# ── Per-scheduler targets ──────────────────────────────────────────

test-cfs: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode test --scheduler cfs --timeout $(TIMEOUT) $(GRADE_FLAGS)

test-rr: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode test --scheduler rr --timeout $(TIMEOUT) $(GRADE_FLAGS)

test-mlfq: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode test --scheduler mlfq --timeout $(TIMEOUT) $(GRADE_FLAGS)

bench-cfs: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode bench --scheduler cfs --timeout $(TIMEOUT) $(GRADE_FLAGS)

bench-rr: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode bench --scheduler rr --timeout $(TIMEOUT) $(GRADE_FLAGS)

bench-mlfq: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode bench --scheduler mlfq --timeout $(TIMEOUT) $(GRADE_FLAGS)

grade-cfs: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --scheduler cfs --timeout $(TIMEOUT) $(GRADE_FLAGS)

grade-rr: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --scheduler rr --timeout $(TIMEOUT) $(GRADE_FLAGS)

grade-mlfq: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --scheduler mlfq --timeout $(TIMEOUT) $(GRADE_FLAGS)

test-o1: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode test --scheduler o1 --timeout $(TIMEOUT) $(GRADE_FLAGS)

bench-o1: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode bench --scheduler o1 --timeout $(TIMEOUT) $(GRADE_FLAGS)

grade-o1: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --scheduler o1 --timeout $(TIMEOUT) $(GRADE_FLAGS)

test-eevdf: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode test --scheduler eevdf --timeout $(TIMEOUT) $(GRADE_FLAGS)

bench-eevdf: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode bench --scheduler eevdf --timeout $(TIMEOUT) $(GRADE_FLAGS)

grade-eevdf: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --scheduler eevdf --timeout $(TIMEOUT) $(GRADE_FLAGS)

test-drl: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode test --scheduler drl --timeout $(TIMEOUT) $(GRADE_FLAGS)

bench-drl: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode bench --scheduler drl --timeout $(TIMEOUT) $(GRADE_FLAGS)

grade-drl: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --scheduler drl --timeout $(TIMEOUT) $(GRADE_FLAGS)

test-hybrid-drl: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode test --scheduler hybrid_drl --timeout $(TIMEOUT) $(GRADE_FLAGS)

bench-hybrid-drl: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode bench --scheduler hybrid_drl --timeout $(TIMEOUT) $(GRADE_FLAGS)

grade-hybrid-drl: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --scheduler hybrid_drl --timeout $(TIMEOUT) $(GRADE_FLAGS)
