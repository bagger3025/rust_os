TARGET  := riscv64gc-unknown-none-elf
PYTHON  := python3
GRADE   := tests/grade_scheduler.py
TIMEOUT := 300

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

.PHONY: all build clean qemu test bench grade \
	test-cfs test-rr test-mlfq test-o1 test-eevdf \
	bench-cfs bench-rr bench-mlfq bench-o1 bench-eevdf \
	grade-cfs grade-rr grade-mlfq grade-o1 grade-eevdf

all: grade

build:
	cargo build --target $(TARGET) $(CARGO_FLAGS)

clean:
	rm -rf target

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

