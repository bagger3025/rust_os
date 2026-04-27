// ============================================================================
// Hybrid DRL — EEVDF with bounded learned deadline shaping
// ============================================================================
//
// EEVDF remains the safety baseline: eligibility is still based on vruntime,
// and the fallback path still picks the lowest vruntime task. A tiny
// fixed-point learner can only apply a small signed adjustment to an eligible
// task's virtual deadline, letting it prefer likely wakeup/interactive tasks
// without breaking EEVDF's fairness and starvation bounds.
// ============================================================================

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use kernel::{
    param::NPROC,
    prng::XorShift64,
    proc::{ProcState, CPUS, PROCS},
    riscv::intr_on,
    swtch::swtch,
};

use super::InstanceScheduler;

const NICE_0_WEIGHT: u64 = 1024;
const SCHED_SLICE: u64 = 4;
const SCHED_MIN_GRANULARITY: u64 = 1;
const WAKEUP_LAG_LIMIT: u64 = SCHED_SLICE;
const ELIGIBILITY_SLACK: u64 = 1;

const NUM_FEATURES: usize = 6;
const HIDDEN_SIZE: usize = 8;
const SCALE: i64 = 1024;
const WAIT_CLAMP: u64 = 100;
const LAG_CLAMP: u64 = 16;
const DEADLINE_CLAMP: u64 = 16;
const RUN_BURST_CLAMP: u64 = 16;
const MAX_WAKE_CREDIT: u64 = 6;
const WAKE_CREDIT_STEP: u64 = 3;
const MAX_DRL_ADJ: i64 = 3;
const EPOCH_LEN: u64 = 128;
const PERTURB_MAG: i64 = 8;

const STATE_UNUSED: u8 = 0;
const STATE_SLEEPING: u8 = 2;
const STATE_RUNNABLE: u8 = 3;
const STATE_RUNNING: u8 = 4;

const VRT_ZERO: AtomicU64 = AtomicU64::new(0);
const STATE_UNUSED_ATOMIC: AtomicU8 = AtomicU8::new(STATE_UNUSED);

static VRUNTIME: [AtomicU64; NPROC] = [VRT_ZERO; NPROC];
static DEADLINE: [AtomicU64; NPROC] = [VRT_ZERO; NPROC];
static LAST_RUN: [AtomicU64; NPROC] = [VRT_ZERO; NPROC];
static RUN_COUNT: [AtomicU64; NPROC] = [VRT_ZERO; NPROC];
static RUN_BURST: [AtomicU64; NPROC] = [VRT_ZERO; NPROC];
static WAKE_CREDIT: [AtomicU64; NPROC] = [VRT_ZERO; NPROC];
static LAST_STATE: [AtomicU8; NPROC] = [STATE_UNUSED_ATOMIC; NPROC];
static MIN_VRUNTIME: AtomicU64 = AtomicU64::new(0);
static ROUND: AtomicU64 = AtomicU64::new(1);
static SEED_CTR: AtomicU64 = AtomicU64::new(0);

#[inline]
fn calc_delta(delta_exec: u64, weight: u64) -> u64 {
    delta_exec * NICE_0_WEIGHT / weight.max(1)
}

#[inline]
fn calc_slice_vruntime(weight: u64) -> u64 {
    SCHED_SLICE * NICE_0_WEIGHT / weight.max(1)
}

#[inline]
fn advance_min_vruntime(candidate: u64) {
    let _ = MIN_VRUNTIME.fetch_update(Ordering::Release, Ordering::Relaxed, |cur| {
        if candidate > cur {
            Some(candidate)
        } else {
            None
        }
    });
}

#[inline]
fn state_code(state: ProcState) -> u8 {
    match state {
        ProcState::UNUSED => STATE_UNUSED,
        ProcState::USED => 1,
        ProcState::SLEEPING => STATE_SLEEPING,
        ProcState::RUNNABLE => STATE_RUNNABLE,
        ProcState::RUNNING => STATE_RUNNING,
        ProcState::ZOMBIE => 5,
    }
}

#[inline]
fn clamp_scale(value: u64, max: u64) -> i64 {
    value.min(max) as i64 * SCALE / max.max(1) as i64
}

#[inline]
fn apply_deadline_adjustment(deadline: u64, adj: i64) -> u64 {
    if adj < 0 {
        deadline.saturating_sub((-adj) as u64)
    } else {
        deadline.saturating_add(adj as u64)
    }
}

pub struct HybridDrl {
    w1: [[i64; NUM_FEATURES]; HIDDEN_SIZE],
    b1: [i64; HIDDEN_SIZE],
    w2: [i64; HIDDEN_SIZE],
    b2: i64,
    pw1: [[i64; NUM_FEATURES]; HIDDEN_SIZE],
    pb1: [i64; HIDDEN_SIZE],
    pw2: [i64; HIDDEN_SIZE],
    pb2: i64,
    epoch_dispatches: u64,
    epoch_max_wait: u64,
    epoch_max_lag: u64,
    epoch_dispatch_mask: u64,
    prev_reward: i64,
    rng: XorShift64,
}

impl Default for HybridDrl {
    fn default() -> Self {
        let mut w1 = [[0; NUM_FEATURES]; HIDDEN_SIZE];
        w1[0] = [640, 320, 640, 256, 384, -256];
        w1[1] = [256, 768, 128, 512, 128, -128];
        w1[2] = [0, 0, 0, 0, 0, 768];

        let mut w2 = [0; HIDDEN_SIZE];
        w2[0] = 512;
        w2[1] = 384;
        w2[2] = -384;

        let seed = SEED_CTR
            .fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed)
            .wrapping_add(0xD1B5_4A32_D192_ED03);

        Self {
            w1,
            b1: [0; HIDDEN_SIZE],
            w2,
            b2: 0,
            pw1: [[0; NUM_FEATURES]; HIDDEN_SIZE],
            pb1: [0; HIDDEN_SIZE],
            pw2: [0; HIDDEN_SIZE],
            pb2: 0,
            epoch_dispatches: 0,
            epoch_max_wait: 0,
            epoch_max_lag: 0,
            epoch_dispatch_mask: 0,
            prev_reward: 0,
            rng: XorShift64::new(seed),
        }
    }
}

impl HybridDrl {
    #[inline]
    fn rand_perturb(&mut self) -> i64 {
        self.rng.next_i64_inclusive(-PERTURB_MAG, PERTURB_MAG)
    }

    fn forward(&self, f: &[i64; NUM_FEATURES]) -> i64 {
        let mut hidden = [0i64; HIDDEN_SIZE];

        for (j, h) in hidden.iter_mut().enumerate() {
            let mut sum = self.b1[j];
            for (k, value) in f.iter().enumerate() {
                sum = sum.saturating_add(self.w1[j][k].saturating_mul(*value) / SCALE);
            }
            *h = sum.max(0);
        }

        let mut score = self.b2;
        for (j, value) in hidden.iter().enumerate() {
            score = score.saturating_add(self.w2[j].saturating_mul(*value) / SCALE);
        }
        score
    }

    fn apply_perturbation(&mut self) {
        for j in 0..HIDDEN_SIZE {
            for k in 0..NUM_FEATURES {
                let delta = self.rand_perturb();
                self.pw1[j][k] = delta;
                self.w1[j][k] = self.w1[j][k].saturating_add(delta);
            }

            let b_delta = self.rand_perturb();
            self.pb1[j] = b_delta;
            self.b1[j] = self.b1[j].saturating_add(b_delta);

            let w_delta = self.rand_perturb();
            self.pw2[j] = w_delta;
            self.w2[j] = self.w2[j].saturating_add(w_delta);
        }

        let b2_delta = self.rand_perturb();
        self.pb2 = b2_delta;
        self.b2 = self.b2.saturating_add(b2_delta);
    }

    fn revert_perturbation(&mut self) {
        for j in 0..HIDDEN_SIZE {
            for k in 0..NUM_FEATURES {
                self.w1[j][k] = self.w1[j][k].saturating_sub(self.pw1[j][k]);
                self.pw1[j][k] = 0;
            }
            self.b1[j] = self.b1[j].saturating_sub(self.pb1[j]);
            self.pb1[j] = 0;
            self.w2[j] = self.w2[j].saturating_sub(self.pw2[j]);
            self.pw2[j] = 0;
        }
        self.b2 = self.b2.saturating_sub(self.pb2);
        self.pb2 = 0;
    }

    fn learn(&mut self) {
        let coverage = self.epoch_dispatch_mask.count_ones() as i64;
        let reward = (self.epoch_dispatches as i64)
            .saturating_mul(8)
            .saturating_add(coverage.saturating_mul(16))
            .saturating_sub((self.epoch_max_wait as i64).saturating_mul(16))
            .saturating_sub((self.epoch_max_lag as i64).saturating_mul(4));

        if self.prev_reward != 0 && reward <= self.prev_reward {
            self.revert_perturbation();
        }

        self.apply_perturbation();
        self.prev_reward = reward;
        self.epoch_dispatches = 0;
        self.epoch_max_wait = 0;
        self.epoch_max_lag = 0;
        self.epoch_dispatch_mask = 0;
    }

    fn observe_state(idx: usize, state: ProcState) {
        let code = state_code(state);
        let prev = LAST_STATE[idx].swap(code, Ordering::AcqRel);

        if prev == STATE_SLEEPING && code == STATE_RUNNABLE {
            let _ = WAKE_CREDIT[idx].fetch_update(Ordering::AcqRel, Ordering::Relaxed, |credit| {
                Some(credit.saturating_add(WAKE_CREDIT_STEP).min(MAX_WAKE_CREDIT))
            });
            RUN_BURST[idx].store(0, Ordering::Release);
        }
    }

    fn features(
        idx: usize,
        round: u64,
        avg_vrt: u64,
        vrt: u64,
        dl: u64,
    ) -> ([i64; NUM_FEATURES], u64, u64) {
        let last = LAST_RUN[idx].load(Ordering::Relaxed);
        let count = RUN_COUNT[idx].load(Ordering::Relaxed);
        let wait = round.saturating_sub(last);
        let share = count
            .saturating_mul(SCALE as u64)
            .checked_div(round.max(1))
            .unwrap_or(0)
            .min(SCALE as u64);
        let deficit = SCALE - share as i64;
        let lag = avg_vrt.saturating_sub(vrt);
        let wake = WAKE_CREDIT[idx].load(Ordering::Relaxed);
        let deadline_slack = dl.saturating_sub(vrt);
        let run_burst = RUN_BURST[idx].load(Ordering::Relaxed);

        (
            [
                clamp_scale(wait, WAIT_CLAMP),
                deficit,
                clamp_scale(wake, MAX_WAKE_CREDIT),
                clamp_scale(lag, LAG_CLAMP),
                SCALE - clamp_scale(deadline_slack, DEADLINE_CLAMP),
                clamp_scale(run_burst, RUN_BURST_CLAMP),
            ],
            wait,
            lag,
        )
    }

    fn deadline_adjustment(&self, features: &[i64; NUM_FEATURES]) -> i64 {
        let raw = self.forward(features) / SCALE;
        (-raw).clamp(-MAX_DRL_ADJ, MAX_DRL_ADJ)
    }
}

impl InstanceScheduler for HybridDrl {
    fn instance_scheduler(&mut self) -> ! {
        let c = unsafe { CPUS.mycpu() };
        self.apply_perturbation();

        loop {
            intr_on();

            let round = ROUND.load(Ordering::Relaxed);
            let mut sum_vrt: u64 = 0;
            let mut count: u64 = 0;
            let mut active_min_vrt: u64 = u64::MAX;

            for (i, p) in PROCS.pool.iter().enumerate() {
                let inner = p.inner.lock();
                Self::observe_state(i, inner.state);
                match inner.state {
                    ProcState::RUNNABLE | ProcState::RUNNING => {
                        let vrt = VRUNTIME[i].load(Ordering::Relaxed);
                        sum_vrt = sum_vrt.saturating_add(vrt);
                        count += 1;
                        if vrt < active_min_vrt {
                            active_min_vrt = vrt;
                        }
                    }
                    _ => {}
                }
            }

            if active_min_vrt < u64::MAX {
                advance_min_vruntime(active_min_vrt);
            }

            let avg_vrt = if count > 0 { sum_vrt / count } else { 0 };
            let mut best_idx: Option<usize> = None;
            let mut best_dl: u64 = u64::MAX;
            let mut fallback_idx: Option<usize> = None;
            let mut fallback_vrt: u64 = u64::MAX;

            for (i, p) in PROCS.pool.iter().enumerate() {
                let inner = p.inner.lock();
                Self::observe_state(i, inner.state);
                if inner.state == ProcState::RUNNABLE {
                    let vrt = VRUNTIME[i].load(Ordering::Relaxed);
                    let dl = DEADLINE[i].load(Ordering::Relaxed);
                    let (features, _, _) = Self::features(i, round, avg_vrt, vrt, dl);
                    let effective_dl =
                        apply_deadline_adjustment(dl, self.deadline_adjustment(&features));

                    if vrt <= avg_vrt.saturating_add(ELIGIBILITY_SLACK) && effective_dl < best_dl {
                        best_dl = effective_dl;
                        best_idx = Some(i);
                    }

                    if vrt < fallback_vrt {
                        fallback_vrt = vrt;
                        fallback_idx = Some(i);
                    }
                }
            }

            if let Some(idx) = best_idx.or(fallback_idx) {
                let p = &PROCS.pool[idx];
                let mut inner = p.inner.lock();

                if inner.state == ProcState::RUNNABLE {
                    let floor = MIN_VRUNTIME
                        .load(Ordering::Relaxed)
                        .saturating_sub(WAKEUP_LAG_LIMIT);
                    let cur_vrt = VRUNTIME[idx].load(Ordering::Relaxed);
                    if cur_vrt < floor {
                        VRUNTIME[idx].store(floor, Ordering::Relaxed);
                    }

                    let vrt = VRUNTIME[idx].load(Ordering::Relaxed);
                    let dl = DEADLINE[idx].load(Ordering::Relaxed);
                    let (_, wait, lag) = Self::features(idx, round, avg_vrt, vrt, dl);
                    self.epoch_dispatches = self.epoch_dispatches.saturating_add(1);
                    self.epoch_max_wait = self.epoch_max_wait.max(wait);
                    self.epoch_max_lag = self.epoch_max_lag.max(lag);
                    self.epoch_dispatch_mask |= 1u64 << idx;

                    if dl <= vrt {
                        DEADLINE[idx]
                            .store(vrt + calc_slice_vruntime(NICE_0_WEIGHT), Ordering::Relaxed);
                    }

                    inner.state = ProcState::RUNNING;
                    unsafe {
                        (*c).proc.replace(Arc::clone(p));
                        swtch(&mut (*c).context, &p.data().context);
                        (*c).proc.take();
                    }

                    let delta = calc_delta(SCHED_MIN_GRANULARITY, NICE_0_WEIGHT);
                    let new_vrt = VRUNTIME[idx].fetch_add(delta, Ordering::Release) + delta;
                    let new_round = ROUND.fetch_add(1, Ordering::AcqRel).saturating_add(1);
                    LAST_RUN[idx].store(new_round, Ordering::Release);
                    RUN_COUNT[idx].fetch_add(1, Ordering::AcqRel);
                    RUN_BURST[idx].fetch_add(1, Ordering::AcqRel);
                    WAKE_CREDIT[idx]
                        .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |credit| {
                            Some(credit.saturating_sub(1))
                        })
                        .ok();

                    let dl = DEADLINE[idx].load(Ordering::Relaxed);
                    if new_vrt >= dl {
                        DEADLINE[idx].store(
                            new_vrt + calc_slice_vruntime(NICE_0_WEIGHT),
                            Ordering::Relaxed,
                        );
                    }

                    if self.epoch_dispatches >= EPOCH_LEN {
                        self.learn();
                    }
                }
            }
        }
    }
}
