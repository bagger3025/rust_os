TARGET  := riscv64gc-unknown-none-elf
KERNEL  := target/$(TARGET)/debug/octox
FS_IMG  := target/fs.img
PYTHON  := python3
GRADE   := tests/grade_scheduler.py
TIMEOUT := 300

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
	cargo build --target $(TARGET)

clean:
	rm -rf target

# ── Run in QEMU (interactive) ──────────────────────────────────────

qemu: build
	$(QEMU) $(QEMUOPTS) -kernel $(KERNEL)

# ── Full suite (all schedulers) ────────────────────────────────────

grade: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --scheduler all --timeout $(TIMEOUT)

test: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode test --scheduler all --timeout $(TIMEOUT)

bench: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode bench --scheduler all --timeout $(TIMEOUT)

# ── Per-scheduler targets ──────────────────────────────────────────

test-cfs: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode test --scheduler cfs --timeout $(TIMEOUT)

test-rr: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode test --scheduler rr --timeout $(TIMEOUT)

test-mlfq: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode test --scheduler mlfq --timeout $(TIMEOUT)

bench-cfs: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode bench --scheduler cfs --timeout $(TIMEOUT)

bench-rr: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode bench --scheduler rr --timeout $(TIMEOUT)

bench-mlfq: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode bench --scheduler mlfq --timeout $(TIMEOUT)

grade-cfs: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --scheduler cfs --timeout $(TIMEOUT)

grade-rr: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --scheduler rr --timeout $(TIMEOUT)

grade-mlfq: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --scheduler mlfq --timeout $(TIMEOUT)

test-o1: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode test --scheduler o1 --timeout $(TIMEOUT)

bench-o1: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode bench --scheduler o1 --timeout $(TIMEOUT)

grade-o1: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --scheduler o1 --timeout $(TIMEOUT)

test-eevdf: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode test --scheduler eevdf --timeout $(TIMEOUT)

bench-eevdf: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --mode bench --scheduler eevdf --timeout $(TIMEOUT)

grade-eevdf: build
	@pkill -9 -f $(QEMU) 2>/dev/null || true
	$(PYTHON) $(GRADE) --scheduler eevdf --timeout $(TIMEOUT)

