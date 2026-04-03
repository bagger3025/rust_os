#!/usr/bin/env python3
"""
grade_scheduler.py — Correctness tests and benchmarks for Octox schedulers.

Usage:
    python3 tests/grade_scheduler.py                        # run everything
    python3 tests/grade_scheduler.py --mode test            # only tests
    python3 tests/grade_scheduler.py --mode bench           # only benchmarks
    python3 tests/grade_scheduler.py --scheduler {model}    # only {model}
    python3 tests/grade_scheduler.py fairness               # filter by name
"""

import re, math
from octoxtest import (
    test, benchmark, run_all,
    parse_bench_output, assert_lines_match,
)

# ====================================================================
#  Correctness Tests
# ====================================================================

@test(5, "boot")
def test_boot(qemu):
    """OS boots and reaches shell prompt."""
    assert_lines_match(qemu.output, r"octox kernel is booting")


@test(5, "echo")
def test_echo(qemu):
    """Shell echo command works."""
    qemu.run_script(["echo hello world"])
    assert_lines_match(qemu.output, r"hello world")


@test(10, "fork and wait")
def test_fork_wait(qemu):
    """benchthruput runs: fork/exit/wait cycle works."""
    qemu.run_script(["benchthruput"], timeout=180)
    data = parse_bench_output(qemu.output)
    assert "throughput" in data, "No BENCH:throughput output found"
    count = int(data["throughput"][0]["count"])
    assert count > 0, "throughput count must be > 0, got %d" % count


@test(10, "concurrent processes")
def test_concurrent(qemu):
    """benchfair forks 4 children that all complete."""
    qemu.run_script(["benchfair"], timeout=180)
    data = parse_bench_output(qemu.output)
    assert "fairness" in data, "No BENCH:fairness output found"
    entries = data["fairness"]
    assert len(entries) == 4, "Expected 4 fairness entries, got %d" % len(entries)
    for e in entries:
        c = int(e["count"])
        assert c > 0, "fairness count must be > 0 for pid=%s" % e.get("pid")


@test(10, "sleep correctness")
def test_sleep(qemu):
    """sleep(5) takes approximately 5 ticks."""
    qemu.run_script(["benchlatency"], timeout=180)
    data = parse_bench_output(qemu.output)
    assert "sleep" in data, "No BENCH:sleep output found"
    elapsed = int(data["sleep"][0]["elapsed"])
    assert 5 <= elapsed <= 8, \
        "sleep(5) elapsed should be 5-8 ticks, got %d" % elapsed


@test(10, "preemption")
def test_preemption(qemu):
    """I/O-bound processes complete under CPU load (preemption works)."""
    qemu.run_script(["benchiobound"], timeout=180)
    data = parse_bench_output(qemu.output)
    assert "iobound" in data, "No BENCH:iobound output found"
    # At least some wakeup measurements should exist
    assert len(data["iobound"]) >= 3, \
        "Expected >= 3 iobound entries, got %d" % len(data["iobound"])


@test(10, "process cleanup")
def test_proc_cleanup(qemu):
    """Processes are recycled beyond NPROC (fork+exit > 64 times)."""
    qemu.run_script(["benchthruput"], timeout=180)
    data = parse_bench_output(qemu.output)
    assert "throughput" in data, "No BENCH:throughput output found"
    count = int(data["throughput"][0]["count"])
    assert count > 64, \
        "throughput count should exceed NPROC=64 (got %d), proving slot reuse" % count


@test(5, "max procs scaling")
def test_max_procs(qemu):
    """benchoverhed with n=8 completes within NPROC=64."""
    qemu.run_script(["benchoverhed"], timeout=300)
    data = parse_bench_output(qemu.output)
    assert "overhead" in data, "No BENCH:overhead output found"
    # Check that n=8 entries exist
    n8_entries = [e for e in data["overhead"] if e.get("n") == "8"]
    assert len(n8_entries) == 8, \
        "Expected 8 entries for n=8, got %d" % len(n8_entries)


@test(5, "symmetric CPU-bound")
def test_cpusym(qemu):
    """benchcpusym: 8 CPU-bound children complete and report total work."""
    qemu.run_script(["benchcpusym"], timeout=180)
    data = parse_bench_output(qemu.output)
    assert "cpusym" in data, "No BENCH:cpusym output found"
    total = int(data["cpusym"][0]["total_work"])
    assert total > 0, "cpusym total_work must be > 0, got %d" % total


@test(5, "sleeper wakeup")
def test_sleepw(qemu):
    """benchsleepw: sleeper child completes under CPU load."""
    qemu.run_script(["benchsleepw"], timeout=180)
    data = parse_bench_output(qemu.output)
    assert "sleepw" in data, "No BENCH:sleepw output found"
    # avg_delay and max_delay should be present
    assert "avg_delay" in data["sleepw"][0], "Missing avg_delay field"
    assert "max_delay" in data["sleepw"][0], "Missing max_delay field"


@test(5, "new process latency")
def test_newproc(qemu):
    """benchnewprc: newly forked processes get scheduled under CPU load."""
    qemu.run_script(["benchnewprc"], timeout=180)
    data = parse_bench_output(qemu.output)
    assert "newproc" in data, "No BENCH:newproc output found"
    assert "avg_latency" in data["newproc"][0], "Missing avg_latency field"


@test(5, "virtual deadline bound")
def test_vdl(qemu):
    """benchvdl: 8 children complete and report scheduling gaps."""
    qemu.run_script(["benchvdl"], timeout=180)
    data = parse_bench_output(qemu.output)
    assert "vdl" in data, "No BENCH:vdl output found"
    max_gap = int(data["vdl"][0]["max_gap"])
    assert max_gap >= 0, "vdl max_gap must be >= 0"


@test(5, "interactive/batch separation")
def test_epoch(qemu):
    """benchepoch: interactive and batch children both complete."""
    qemu.run_script(["benchepoch"], timeout=180)
    data = parse_bench_output(qemu.output)
    assert "epoch" in data, "No BENCH:epoch output found"
    assert "io_resp" in data["epoch"][0], "Missing io_resp field"
    assert "batch_work" in data["epoch"][0], "Missing batch_work field"
    batch_work = int(data["epoch"][0]["batch_work"])
    assert batch_work > 0, "batch_work must be > 0"


@test(5, "adaptive workload")
def test_adapt(qemu):
    """benchadapt: phase-changing workload completes."""
    qemu.run_script(["benchadapt"], timeout=180)
    data = parse_bench_output(qemu.output)
    assert "adapt" in data, "No BENCH:adapt output found"
    assert "phase2_delay" in data["adapt"][0], "Missing phase2_delay field"
    assert "phase2_work" in data["adapt"][0], "Missing phase2_work field"
    phase2_work = int(data["adapt"][0]["phase2_work"])
    assert phase2_work > 0, "phase2_work must be > 0"


@test(5, "heavy fairness")
def test_fairhvy(qemu):
    """benchfairhvy: 12 CPU-bound children complete with non-zero work."""
    qemu.run_script(["benchfairhvy"], timeout=300)
    data = parse_bench_output(qemu.output)
    assert "fairhvy" in data, "No BENCH:fairhvy output found"
    entries = data["fairhvy"]
    assert len(entries) == 12, "Expected 12 fairhvy entries, got %d" % len(entries)
    for e in entries:
        c = int(e["count"])
        assert c > 0, "fairhvy count must be > 0 for pid=%s" % e.get("pid")


@test(5, "convoy effect")
def test_convoy(qemu):
    """benchconvoy: hogs and burst workers both complete."""
    qemu.run_script(["benchconvoy"], timeout=300)
    data = parse_bench_output(qemu.output)
    assert "convoy" in data, "No BENCH:convoy output found"
    assert "hog_work" in data["convoy"][0], "Missing hog_work field"
    assert "burst_delay" in data["convoy"][0], "Missing burst_delay field"


@test(5, "starvation resistance")
def test_starve(qemu):
    """benchstarve: all 12 processes complete with non-zero work."""
    qemu.run_script(["benchstarve"], timeout=300)
    data = parse_bench_output(qemu.output)
    assert "starve" in data, "No BENCH:starve output found"
    entries = data["starve"]
    assert len(entries) == 12, "Expected 12 starve entries, got %d" % len(entries)
    for e in entries:
        c = int(e["count"])
        assert c > 0, "starve count must be > 0 for pid=%s" % e.get("pid")


@test(5, "heavy I/O scheduling")
def test_iosched(qemu):
    """benchiosched: I/O workers complete under heavy CPU load."""
    qemu.run_script(["benchiosched"], timeout=300)
    data = parse_bench_output(qemu.output)
    assert "iosched" in data, "No BENCH:iosched output found"
    assert len(data["iosched"]) >= 2, \
        "Expected >= 2 iosched entries, got %d" % len(data["iosched"])


@test(5, "heavy tail latency")
def test_vdlhvy(qemu):
    """benchvdlhvy: 12 CPU-bound children report scheduling gaps."""
    qemu.run_script(["benchvdlhvy"], timeout=300)
    data = parse_bench_output(qemu.output)
    assert "vdlhvy" in data, "No BENCH:vdlhvy output found"
    max_gap = int(data["vdlhvy"][0]["max_gap"])
    assert max_gap >= 0, "vdlhvy max_gap must be >= 0"


# ====================================================================
#  Benchmarks
# ====================================================================

@benchmark("throughput")
def benchthruput(qemu, sched):
    """Fork/exit throughput: ops per tick."""
    qemu.run_script(["benchthruput"], timeout=180)
    data = parse_bench_output(qemu.output)
    if "throughput" not in data:
        return {}
    count = float(data["throughput"][0]["count"])
    ticks = float(data["throughput"][0]["ticks"])
    ops_per_tick = count / max(ticks, 1)
    return {"throughput (ops/tick)": ops_per_tick}


@benchmark("fairness")
def benchfair(qemu, sched):
    """CPU share fairness: coefficient of variation (lower = fairer)."""
    qemu.run_script(["benchfair"], timeout=180)
    data = parse_bench_output(qemu.output)
    if "fairness" not in data:
        return {}
    counts = [float(e["count"]) for e in data["fairness"]]
    if not counts:
        return {}
    mean = sum(counts) / len(counts)
    if mean == 0:
        return {"fairness CV": 0.0}
    variance = sum((c - mean) ** 2 for c in counts) / len(counts)
    cv = math.sqrt(variance) / mean
    return {"fairness CV (lower=better)": cv}


@benchmark("latency")
def benchlatency(qemu, sched):
    """First-slice scheduling latency under load."""
    qemu.run_script(["benchlatency"], timeout=180)
    data = parse_bench_output(qemu.output)
    metrics = {}

    # Idle latency
    if "latency" in data:
        delays = [float(e["delay"]) for e in data["latency"]]
        if delays:
            metrics["latency idle (mean ticks)"] = sum(delays) / len(delays)

    # Loaded latency
    if "latency_loaded" in data:
        delays = [float(e["delay"]) for e in data["latency_loaded"]]
        if delays:
            metrics["latency loaded (mean ticks)"] = sum(delays) / len(delays)

    return metrics


@benchmark("iobound")
def benchiobound(qemu, sched):
    """I/O-bound wakeup responsiveness under CPU load."""
    qemu.run_script(["benchiobound"], timeout=180)
    data = parse_bench_output(qemu.output)
    if "iobound" not in data:
        return {}
    delays = [float(e["wakeup_delay"]) for e in data["iobound"]]
    if not delays:
        return {}
    mean_delay = sum(delays) / len(delays)
    sorted_d = sorted(delays)
    p95_idx = min(int(len(sorted_d) * 0.95), len(sorted_d) - 1)
    return {
        "io wakeup delay mean (lower=better)": mean_delay,
        "io wakeup delay p95": sorted_d[p95_idx],
    }


@benchmark("overhead")
def benchoverhed(qemu, sched):
    """Context switch overhead: throughput scaling with process count."""
    qemu.run_script(["benchoverhed"], timeout=300)
    data = parse_bench_output(qemu.output)
    if "overhead" not in data:
        return {}

    # Group by n (skip malformed entries from interleaved serial output)
    by_n = {}
    for e in data["overhead"]:
        try:
            n = int(e["n"])
            c = float(e["count"])
        except (ValueError, KeyError):
            continue
        by_n.setdefault(n, []).append(c)

    metrics = {}
    for n in sorted(by_n):
        total = sum(by_n[n])
        metrics["overhead total work n=%d" % n] = total

    # Compute ratio: n=8 total / n=1 total  (lower overhead = ratio closer to 1)
    if 1 in by_n and 8 in by_n:
        r = sum(by_n[8]) / max(sum(by_n[1]), 1)
        metrics["overhead scaling ratio n8/n1 (lower=better)"] = r

    return metrics


# ====================================================================
#  Scenario Benchmarks
#  Each targets a specific scheduler's design advantage.
# ====================================================================

@benchmark("cpusym (→RR)")
def benchcpusym(qemu, sched):
    """Symmetric CPU-bound throughput — tests scheduling overhead.
    RoundRobin should win: zero per-process metadata overhead."""
    qemu.run_script(["benchcpusym"], timeout=180)
    data = parse_bench_output(qemu.output)
    if "cpusym" not in data:
        return {}
    total = float(data["cpusym"][0]["total_work"])
    min_work = float(data["cpusym"][0]["min_work"])
    return {
        "cpusym total work": total,
        "cpusym min/child": min_work,
    }


@benchmark("sleepw (→CFS)")
def benchsleepw(qemu, sched):
    """Sleeper wakeup bonus — tests I/O priority boost.
    CFS should win: SLEEPER_BONUS gives waking tasks a vruntime head start."""
    qemu.run_script(["benchsleepw"], timeout=180)
    data = parse_bench_output(qemu.output)
    if "sleepw" not in data:
        return {}
    avg = float(data["sleepw"][0]["avg_delay"])
    mx = float(data["sleepw"][0]["max_delay"])
    return {
        "sleepw avg delay (lower=better)": avg,
        "sleepw max delay (lower=better)": mx,
    }


@benchmark("newproc (→MLFQ)")
def benchnewprc(qemu, sched):
    """New process priority — tests feedback queue advantage.
    MLFQ should win: new tasks start at highest priority level."""
    qemu.run_script(["benchnewprc"], timeout=180)
    data = parse_bench_output(qemu.output)
    if "newproc" not in data:
        return {}
    avg = float(data["newproc"][0]["avg_latency"])
    mx = float(data["newproc"][0]["max_latency"])
    return {
        "newproc avg latency (lower=better)": avg,
        "newproc max latency (lower=better)": mx,
    }


@benchmark("vdl (→EEVDF)")
def benchvdl(qemu, sched):
    """Virtual deadline tail-latency — tests bounded scheduling gaps.
    EEVDF should win: eligibility filter + deadlines minimize worst-case gap."""
    qemu.run_script(["benchvdl"], timeout=180)
    data = parse_bench_output(qemu.output)
    if "vdl" not in data:
        return {}
    max_gap = float(data["vdl"][0]["max_gap"])
    avg_gap = float(data["vdl"][0]["avg_gap_100"]) / 100.0
    spread = float(data["vdl"][0]["spread"]) / 100.0
    return {
        "vdl max gap (lower=better)": max_gap,
        "vdl avg gap": avg_gap,
        "vdl spread (lower=better)": spread,
    }


@benchmark("epoch (→O1)")
def benchepoch(qemu, sched):
    """Interactive/batch separation — tests interactivity heuristic.
    O(1) should win: sleep_avg tracks interactive vs batch behavior."""
    qemu.run_script(["benchepoch"], timeout=180)
    data = parse_bench_output(qemu.output)
    if "epoch" not in data:
        return {}
    io_resp = float(data["epoch"][0]["io_resp"])
    batch_work = float(data["epoch"][0]["batch_work"])
    return {
        "epoch io overshoot (lower=better)": io_resp,
        "epoch batch work": batch_work,
    }


@benchmark("adapt")
def benchadapt(qemu, sched):
    """Adaptive workload phase change — tests scheduler adaptation."""
    qemu.run_script(["benchadapt"], timeout=180)
    data = parse_bench_output(qemu.output)
    if "adapt" not in data:
        return {}
    p2_delay = float(data["adapt"][0]["phase2_delay"])
    p2_work = float(data["adapt"][0]["phase2_work"])
    return {
        "adapt phase2 delay (lower=better)": p2_delay,
        "adapt phase2 cpu work": p2_work,
    }


# ====================================================================
#  Heavy Contention Benchmarks
#  Higher process counts to expose real scheduling differences.
# ====================================================================

@benchmark("fairhvy (heavy fairness)")
def benchfairhvy(qemu, sched):
    """Heavy fairness: 12 CPU-bound on 4 cores (3x oversubscription).
    CFS/EEVDF should win: vruntime tracking guarantees proportional CPU."""
    qemu.run_script(["benchfairhvy"], timeout=300)
    data = parse_bench_output(qemu.output)
    if "fairhvy" not in data:
        return {}
    counts = [float(e["count"]) for e in data["fairhvy"]]
    if not counts:
        return {}
    mean = sum(counts) / len(counts)
    if mean == 0:
        return {"heavy fairness CV (lower=better)": 0.0}
    variance = sum((c - mean) ** 2 for c in counts) / len(counts)
    cv = math.sqrt(variance) / mean
    mn = min(counts)
    mx = max(counts)
    return {
        "heavy fairness CV (lower=better)": cv,
        "heavy fairness min/max": mn / mx if mx > 0 else 0,
    }


@benchmark("convoy (→MLFQ)")
def benchconvoy(qemu, sched):
    """Convoy effect: burst worker wakeup delay behind CPU hogs.
    MLFQ should win: hogs demoted, burst workers stay at top priority."""
    qemu.run_script(["benchconvoy"], timeout=300)
    data = parse_bench_output(qemu.output)
    if "convoy" not in data:
        return {}
    d = float(data["convoy"][0]["burst_delay"])
    w = float(data["convoy"][0]["hog_work"])
    return {
        "convoy burst delay (lower=better)": d,
        "convoy hog work": w,
    }


@benchmark("starve (starvation)")
def benchstarve(qemu, sched):
    """Starvation resistance: min/max work ratio across 12 processes.
    CFS/EEVDF should win: proportional fair share prevents starvation."""
    qemu.run_script(["benchstarve"], timeout=300)
    data = parse_bench_output(qemu.output)
    if "starve" not in data:
        return {}
    counts = [float(e["count"]) for e in data["starve"]]
    if not counts:
        return {}
    mn = min(counts)
    mx = max(counts)
    ratio = mn / mx if mx > 0 else 0
    return {
        "starve min/max ratio (higher=better)": ratio,
        "starve min work": mn,
    }


@benchmark("iosched (heavy IO →CFS)")
def benchiosched(qemu, sched):
    """Heavy I/O: 2 sleepers vs 10 CPU workers (2.5x oversubscription).
    CFS should win: SLEEPER_BONUS gives wakers immediate priority."""
    qemu.run_script(["benchiosched"], timeout=300)
    data = parse_bench_output(qemu.output)
    if "iosched" not in data:
        return {}
    delays = [float(e["wakeup_delay"]) for e in data["iosched"]]
    if not delays:
        return {}
    mean_delay = sum(delays) / len(delays)
    max_delay = max(delays)
    return {
        "iosched mean delay (lower=better)": mean_delay,
        "iosched max delay (lower=better)": max_delay,
    }


@benchmark("vdlhvy (heavy VDL →EEVDF)")
def benchvdlhvy(qemu, sched):
    """Heavy tail latency: 12 processes max scheduling gap.
    EEVDF should win: eligibility + deadlines bound worst-case gap."""
    qemu.run_script(["benchvdlhvy"], timeout=300)
    data = parse_bench_output(qemu.output)
    if "vdlhvy" not in data:
        return {}
    max_gap = float(data["vdlhvy"][0]["max_gap"])
    avg_gap = float(data["vdlhvy"][0]["avg_gap_100"]) / 100.0
    spread = float(data["vdlhvy"][0]["spread"]) / 100.0
    return {
        "vdlhvy max gap (lower=better)": max_gap,
        "vdlhvy avg gap": avg_gap,
        "vdlhvy spread (lower=better)": spread,
    }


# ====================================================================
#  Main
# ====================================================================

if __name__ == "__main__":
    run_all()
