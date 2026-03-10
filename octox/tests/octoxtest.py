"""
octoxtest.py — Test and benchmark harness for the Octox OS scheduler.
  - OctoxBuilder : cargo build with scheduler feature-flag selection
  - OctoxQEMU    : launch QEMU, send shell commands, capture output
  - @test / @benchmark decorators and run_all()
  - parse_bench_output() for structured BENCH: lines
  - Colored pass/fail reporting and score tracking
"""

from __future__ import print_function
import sys, os, re, time, select, subprocess, json, math
from subprocess import Popen, PIPE, STDOUT
from optparse import OptionParser

__all__ = [
    "OctoxBuilder", "OctoxQEMU",
    "test", "benchmark", "run_all",
    "parse_bench_output", "assert_lines_match",
    "color",
]

# ── project root (auto-detected) ────────────────────────────────────────
SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
PROJECT_ROOT = os.path.dirname(SCRIPT_DIR)          # octox/
TARGET = "riscv64gc-unknown-none-elf"

# ── global test / benchmark registries ──────────────────────────────────
TESTS = []
BENCHMARKS = []
TOTAL = POSSIBLE = 0
CURRENT_TEST = None
GRADES = {}

# ── options (populated by run_all) ──────────────────────────────────────
options = None


# ====================================================================
#  Builder
# ====================================================================

class OctoxBuilder:
    """Build the Octox kernel with scheduler selection via --cfg flags.

    The scheduler is chosen by passing --cfg sched_xx through RUSTFLAGS.
    main.rs already has cfg-gated imports that select the right scheduler:

        #[cfg(sched_rr)]
        use scheduler::round_robin::RoundRobin as ActiveScheduler;
        ...
        #[cfg(not(any(sched_rr, sched_mlfq, sched_o1, sched_eevdf)))]
        use scheduler::cfs::Cfs as ActiveScheduler;

    No source code patching is needed — only compile flags change.
    """

    # Map scheduler name -> cfg flag (None = default / CFS)
    SCHED_CFG = {
        "cfs":   None,           # default: no flag needed
        "rr":    "sched_rr",
        "mlfq":  "sched_mlfq",
        "o1":    "sched_o1",
        "eevdf": "sched_eevdf",
    }

    MAIN_RS = "src/kernel/main.rs"

    def __init__(self, root=PROJECT_ROOT, release=False):
        self.root = root
        self.release = release
        self._last_cfg = None  # track last RUSTFLAGS to detect changes

    def build(self, scheduler="cfs"):
        """Build kernel with the chosen scheduler. Returns (kernel_path, fs_img_path).

        Uses RUSTFLAGS=--cfg sched_xx to select the scheduler at compile time.
        """
        cfg_flag = self.SCHED_CFG.get(scheduler)

        # When RUSTFLAGS changes between builds, cargo may not detect the
        # change and skip recompilation.  Touch main.rs to force a rebuild.
        if cfg_flag != self._last_cfg:
            main_path = os.path.join(self.root, self.MAIN_RS)
            os.utime(main_path, None)
            self._last_cfg = cfg_flag

        cmd = ["cargo", "build", "--target", TARGET]
        if self.release:
            cmd.append("--release")
        env = os.environ.copy()

        if cfg_flag:
            # Append --cfg to any existing RUSTFLAGS
            existing = env.get("RUSTFLAGS", "")
            env["RUSTFLAGS"] = (existing + " --cfg " + cfg_flag).strip()

        if options and options.verbose:
            if cfg_flag:
                print("$ RUSTFLAGS='--cfg %s' %s" % (cfg_flag, " ".join(cmd)))
            else:
                print("$", " ".join(cmd))

        result = subprocess.run(cmd, cwd=self.root, env=env,
                                stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT)
        if result.returncode != 0:
            print(result.stdout.decode("utf-8", "replace"))
            raise RuntimeError("cargo build failed for scheduler=%s" % scheduler)

        self._last_scheduler = scheduler
        profile = "release" if self.release else "debug"
        kernel = os.path.join(self.root, "target", TARGET, profile, "octox")
        fs_img = os.path.join(self.root, "target", "fs.img")
        assert os.path.exists(kernel), "kernel binary not found: " + kernel
        assert os.path.exists(fs_img), "fs.img not found: " + fs_img
        return kernel, fs_img

    def restore(self):
        """No-op — source code is never modified."""
        pass


# ====================================================================
#  QEMU controller
# ====================================================================

class OctoxQEMU:
    """Launch and interact with QEMU running Octox."""

    QEMU_BIN = "qemu-system-riscv64"

    def __init__(self, kernel_path, fs_img_path):
        cmd = [
            self.QEMU_BIN,
            "-machine", "virt",
            "-bios", "none",
            "-m", "524M",
            "-smp", "4",
            "-nographic",
            "-serial", "mon:stdio",
            "-global", "virtio-mmio.force-legacy=false",
            "-drive", "file=%s,if=none,format=raw,id=x0" % fs_img_path,
            "-device", "virtio-blk-device,drive=x0,bus=virtio-mmio-bus.0",
            "-kernel", kernel_path,
        ]
        if options and options.verbose:
            print("$", " ".join(cmd))
        self.proc = Popen(cmd, stdin=PIPE, stdout=PIPE, stderr=STDOUT)
        self.output = ""
        self.outbytes = bytearray()

    # ── low-level I/O ───────────────────────────────────────────────

    def fileno(self):
        if self.proc and self.proc.stdout:
            return self.proc.stdout.fileno()
        return None

    def _read_chunk(self, timeout=0.1):
        """Read available bytes from QEMU stdout (non-blocking)."""
        fd = self.fileno()
        if fd is None:
            return b""
        rset, _, _ = select.select([fd], [], [], timeout)
        if rset:
            data = os.read(fd, 4096)
            if data:
                self.outbytes.extend(data)
                self.output = self.outbytes.decode("utf-8", "replace")
            return data
        return b""

    def _read_until(self, predicate, timeout=60):
        """Read until predicate(self.output) is True, or timeout."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            remaining = deadline - time.time()
            self._read_chunk(timeout=min(remaining, 0.5))
            if predicate(self.output):
                return True
        return False

    def write(self, text):
        if isinstance(text, str):
            text = text.encode("utf-8")
        self.proc.stdin.write(text)
        self.proc.stdin.flush()

    # ── high-level helpers ──────────────────────────────────────────

    def wait_for_boot(self, timeout=60):
        """Wait until the shell prompt '$ ' appears."""
        ok = self._read_until(lambda o: "$ " in o, timeout=timeout)
        if not ok:
            raise RuntimeError("QEMU did not boot within %ds.\nOutput:\n%s"
                               % (timeout, self.output))

    def run_script(self, commands, timeout=120):
        """Send shell commands one-by-one, waiting for '$ ' between each.
        Returns the accumulated output after all commands finish.

        After wait_for_boot(), the shell has already printed its first
        prompt and is ready for input.  For each command we:
          1. Send the command (the shell is already waiting at '$ ').
          2. Wait for the next '$ ' prompt (command finished).
        """
        prompt_count = self.output.count("$ ")

        for cmd in commands:
            # The shell is already at a '$ ' prompt — send command.
            self.write(cmd + "\n")
            time.sleep(0.05)
            # Wait for the NEXT prompt to appear (command completed).
            target = prompt_count + 1
            if not self._read_until(
                    lambda o: o.count("$ ") >= target, timeout=timeout):
                break
            prompt_count = self.output.count("$ ")

        return self.output

    def kill(self):
        if self.proc:
            try:
                self.proc.kill()  # SIGKILL — QEMU ignores SIGTERM sometimes
                self.proc.wait(timeout=5)
            except Exception:
                pass
            self.proc = None


# ====================================================================
#  Output parsing
# ====================================================================

def parse_bench_output(output):
    """Parse BENCH:<name>:<key>=<val>[:key=val ...] lines.
    Returns dict:  { name: [ {key: val, ...}, ... ] }
    """
    results = {}
    for line in output.splitlines():
        m = re.match(r"BENCH:(\w+):(.*)", line.strip())
        if not m:
            continue
        name = m.group(1)
        rest = m.group(2)
        entry = {}
        for part in rest.split(":"):
            if "=" in part:
                k, v = part.split("=", 1)
                entry[k] = v
        results.setdefault(name, []).append(entry)
    return results


# ====================================================================
#  Assertions
# ====================================================================

def assert_lines_match(text, *regexps, **kw):
    """Assert all regexps match some line.  no=[...] for lines that must NOT
    appear."""
    no = kw.get("no", [])
    lines = text.splitlines()
    missing = list(regexps)
    bad_lines = []
    for line in lines:
        for r in list(missing):
            if re.search(r, line):
                missing.remove(r)
                break
        for r in no:
            if re.search(r, line):
                bad_lines.append(line)
    msgs = []
    if bad_lines:
        msgs.append("Unexpected lines:\n  " + "\n  ".join(bad_lines))
    for r in missing:
        msgs.append("MISSING pattern: '%s'" % r)
    if msgs:
        raise AssertionError("\n".join(msgs))


class AssertionError(Exception):
    pass


# ====================================================================
#  Test / benchmark decorators
# ====================================================================

def test(points, title=None, scheduler=None):
    """Decorator: register a correctness test.
    scheduler: 'cfs', 'rr', or None (run for whichever scheduler the
    runner is currently testing)."""
    def decorator(fn):
        t = title or fn.__name__.replace("test_", "").replace("_", " ")
        fn._test_title = t
        fn._test_points = points
        fn._test_scheduler = scheduler
        TESTS.append(fn)
        return fn
    return decorator


def benchmark(title=None):
    """Decorator: register a benchmark function.
    The function receives (qemu, scheduler_name) and should return a dict
    of metric_name -> numeric_value."""
    def decorator(fn):
        t = title or fn.__name__.replace("bench_", "").replace("_", " ")
        fn._bench_title = t
        BENCHMARKS.append(fn)
        return fn
    return decorator


# ====================================================================
#  Colored output
# ====================================================================

COLORS = {"default": "\033[0m", "red": "\033[31m", "green": "\033[32m",
          "yellow": "\033[33m", "bold": "\033[1m"}

def color(name, text):
    if options and options.color == "never":
        return text
    if options and options.color == "always":
        return COLORS.get(name, "") + text + COLORS["default"]
    # auto
    if sys.stdout.isatty():
        return COLORS.get(name, "") + text + COLORS["default"]
    return text


# ====================================================================
#  Runner
# ====================================================================

def run_all():
    """Main entry point.  Parse CLI, build, run tests and benchmarks."""
    global options, TOTAL, POSSIBLE, GRADES

    parser = OptionParser(usage="usage: %prog [options] [filter ...]")
    parser.add_option("-v", "--verbose", action="store_true",
                      help="print QEMU commands and full output")
    parser.add_option("--color", choices=["never", "always", "auto"],
                      default="auto")
    parser.add_option("--scheduler", choices=["rr", "cfs", "mlfq", "o1", "eevdf", "all"],
                      default="all",
                      help="which scheduler(s) to test (default: all)")
    parser.add_option("--mode", choices=["test", "bench", "all"],
                      default="all",
                      help="run tests, benchmarks, or all (default: all)")
    parser.add_option("--timeout", type="int", default=120,
                      help="per-test timeout in seconds (default: 120)")
    parser.add_option("--results", help="write JSON results to file")
    parser.add_option("--release", action="store_true",
                      help="build with --release (optimized)")
    (options, args) = parser.parse_args()

    filters = [a.lower() for a in args]
    builder = OctoxBuilder(release=getattr(options, 'release', False))

    schedulers = {
        "all":  ["cfs", "rr", "mlfq", "o1", "eevdf"],
        "cfs":  ["cfs"],
        "rr":   ["rr"],
        "mlfq": ["mlfq"],
        "o1":   ["o1"],
        "eevdf": ["eevdf"],
    }[options.scheduler]

    bench_results = {}  # scheduler -> { metric: value }

    for sched in schedulers:
        sched_label = sched.upper()
        print(color("bold", "\n========== Scheduler: %s ==========" % sched_label))

        # Build
        print("Building kernel with scheduler=%s ..." % sched)
        try:
            kernel, fs_img = builder.build(scheduler=sched)
        except RuntimeError as e:
            print(color("red", "BUILD FAILED: %s" % e))
            continue
        print(color("green", "Build OK"))

        # ── Correctness tests ──────────────────────────────────────
        if options.mode in ("test", "all"):
            for fn in TESTS:
                title = "[%s] %s" % (sched_label, fn._test_title)
                if filters and not any(f in title.lower() for f in filters):
                    continue
                if fn._test_scheduler and fn._test_scheduler != sched:
                    continue

                sys.stdout.write("== Test %s == " % title)
                sys.stdout.flush()

                fail = None
                start = time.time()
                qemu = None
                try:
                    qemu = OctoxQEMU(kernel, fs_img)
                    qemu.wait_for_boot(timeout=options.timeout)
                    fn(qemu)
                except (AssertionError, RuntimeError) as e:
                    fail = str(e)
                except Exception as e:
                    fail = "Exception: %s" % e
                finally:
                    if qemu:
                        if options.verbose:
                            print("\n--- QEMU output ---")
                            print(qemu.output[-2000:])
                            print("--- end ---")
                        qemu.kill()

                pts = fn._test_points
                POSSIBLE += pts
                elapsed = time.time() - start
                if fail:
                    print(color("red", "FAIL"), end=" ")
                else:
                    TOTAL += pts
                    print(color("green", "OK"), end=" ")
                if elapsed > 0.5:
                    print("(%.1fs)" % elapsed, end=" ")
                print()
                if fail:
                    # truncate very long failure messages
                    lines = fail.split("\n")
                    if len(lines) > 10:
                        fail = "\n".join(lines[:10]) + "\n... (%d more lines)" % (len(lines) - 10)
                    print("    %s" % fail.replace("\n", "\n    "))
                GRADES[title] = 0 if fail else pts

        # ── Benchmarks ─────────────────────────────────────────────
        if options.mode in ("bench", "all"):
            bench_results[sched] = {}
            for fn in BENCHMARKS:
                title = fn._bench_title
                if filters and not any(f in title.lower() for f in filters):
                    continue

                sys.stdout.write("== Bench [%s] %s == " % (sched_label, title))
                sys.stdout.flush()

                qemu = None
                try:
                    qemu = OctoxQEMU(kernel, fs_img)
                    qemu.wait_for_boot(timeout=options.timeout)
                    metrics = fn(qemu, sched)
                    bench_results[sched].update(metrics or {})
                    print(color("green", "OK"), end=" ")
                except Exception as e:
                    print(color("red", "FAIL: %s" % e), end=" ")
                finally:
                    if qemu:
                        if options.verbose:
                            print("\n--- QEMU output ---")
                            print(qemu.output[-2000:])
                            print("--- end ---")
                        qemu.kill()
                print()

    # ── Comparison table ───────────────────────────────────────────
    if options.mode in ("bench", "all") and len(bench_results) >= 2:
        sched_names = list(bench_results.keys())
        all_keys = sorted(set(k for m in bench_results.values() for k in m.keys()))

        print(color("bold", "\n========== Scheduler Benchmark Comparison =========="))
        header = "%-32s" % "Metric"
        for s in sched_names:
            header += " %12s" % s.upper()
        header += "   Winner"
        print(header)
        print("-" * (32 + 13 * len(sched_names) + 10))

        for key in all_keys:
            row = "%-32s" % key
            vals = {}
            for s in sched_names:
                v = bench_results[s].get(key)
                vals[s] = v
                row += " %12s" % ("%.4f" % v if v is not None else "n/a")

            valid = {s: v for s, v in vals.items() if v is not None}
            if "(lower=better)" in key.lower():
                lower_better = True
            elif "(higher=better)" in key.lower():
                lower_better = False
            else:
                lower_better = any(w in key.lower()
                                   for w in ["latency", "delay", "cv", "overhead",
                                             "gap", "spread", "overshoot"])
            winner = ""
            if len(valid) >= 2:
                if lower_better:
                    winner = min(valid, key=valid.get).upper()
                else:
                    winner = max(valid, key=valid.get).upper()
            row += "   %s" % winner
            print(row)

    # ── Score summary ──────────────────────────────────────────────
    if options.mode in ("test", "all"):
        print(color("bold", "\nScore: %d/%d" % (TOTAL, POSSIBLE)))

    if options.results:
        try:
            with open(options.results, "w") as f:
                json.dump({"grades": GRADES, "benchmarks": bench_results}, f,
                          indent=2)
            print("Results written to %s" % options.results)
        except Exception as e:
            print("Failed to write results: %s" % e)

    # Restore original main.rs
    builder.restore()

    if TOTAL < POSSIBLE:
        sys.exit(1)
