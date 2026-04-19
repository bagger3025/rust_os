// ============================================================================
// DRL — lightweight online learning scheduler
// ============================================================================
//
// This scheduler scores each RUNNABLE process with a tiny fixed-point neural
// network. It uses a conservative warm start and a simple evolution-strategy
// update at epoch boundaries: keep a perturbation if the previous epoch's
// reward improved, otherwise revert it and try another.
// ============================================================================

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};
use kernel::{
    param::NPROC,
    prng::XorShift64,
    proc::{ProcState, CPUS, PROCS},
    riscv::intr_on,
    swtch::swtch,
};

use super::InstanceScheduler;

const NUM_FEATURES: usize = 4;
const HIDDEN_SIZE: usize = 8;
const SCALE: i64 = 1024;
const EPOCH_LEN: u64 = 64;
const PERTURB_MAG: i64 = 24;
const WAIT_CLAMP: u64 = 100;

const U64_ZERO: AtomicU64 = AtomicU64::new(0);

static LAST_RUN: [AtomicU64; NPROC] = [U64_ZERO; NPROC];
static RUN_COUNT: [AtomicU64; NPROC] = [U64_ZERO; NPROC];
static ROUND: AtomicU64 = AtomicU64::new(1);
static SEED_CTR: AtomicU64 = AtomicU64::new(0);

pub struct Drl {
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
    epoch_dispatch_mask: u64,
    prev_reward: i64,
    rng: XorShift64,
    scan_offset: usize,
}

impl Default for Drl {
    fn default() -> Self {
        let mut w1 = [[0; NUM_FEATURES]; HIDDEN_SIZE];
        w1[0] = [768, 256, 256, 0];
        w1[1] = [256, 768, 0, 0];
        w1[2] = [128, 0, 512, 0];

        let mut w2 = [0; HIDDEN_SIZE];
        w2[0] = 512;
        w2[1] = 384;
        w2[2] = 256;

        let seed = SEED_CTR
            .fetch_add(0xCAFE_BABE_0000_1234, Ordering::Relaxed)
            .wrapping_add(0xDEAD_BEEF_0000_1337);

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
            epoch_dispatch_mask: 0,
            prev_reward: 0,
            rng: XorShift64::new(seed),
            scan_offset: 0,
        }
    }
}

impl Drl {
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
        let throughput_score = if self.epoch_max_wait == 0 {
            (self.epoch_dispatches as i64).saturating_mul(SCALE)
        } else {
            (self.epoch_dispatches as i64).saturating_mul(SCALE) / self.epoch_max_wait as i64
        };
        let fairness_bonus =
            (self.epoch_dispatch_mask.count_ones() as i64).saturating_mul(SCALE) / NPROC as i64;
        let reward = throughput_score.saturating_add(fairness_bonus);

        if self.prev_reward > 0 && reward <= self.prev_reward {
            self.revert_perturbation();
        }

        self.apply_perturbation();
        self.prev_reward = reward;
        self.epoch_dispatches = 0;
        self.epoch_max_wait = 0;
        self.epoch_dispatch_mask = 0;
    }

    fn features(idx: usize, round: u64, total: u64) -> ([i64; NUM_FEATURES], u64) {
        let last = LAST_RUN[idx].load(Ordering::Relaxed);
        let count = RUN_COUNT[idx].load(Ordering::Relaxed);
        let wait = round.saturating_sub(last);
        let wait_score = wait.min(WAIT_CLAMP) as i64 * SCALE / WAIT_CLAMP as i64;
        let share = (count.saturating_mul(SCALE as u64) / total.max(1)).min(SCALE as u64);
        let deficit = SCALE - share as i64;
        let io_hint = if wait > 5 && count > 0 { SCALE } else { 0 };

        ([wait_score, deficit, io_hint, SCALE / 2], wait)
    }
}

impl InstanceScheduler for Drl {
    fn instance_scheduler(&mut self) -> ! {
        let c = unsafe { CPUS.mycpu() };
        self.apply_perturbation();

        loop {
            intr_on();

            let round = ROUND.load(Ordering::Relaxed);
            let total = round.max(1);
            let mut best_idx: Option<usize> = None;
            let mut best_score = i64::MIN;

            for j in 0..NPROC {
                let i = (self.scan_offset + j) % NPROC;
                let p = &PROCS.pool[i];
                let inner = p.inner.lock();
                if inner.state == ProcState::RUNNABLE {
                    let (features, _) = Self::features(i, round, total);
                    let score = self.forward(&features);
                    if score > best_score {
                        best_score = score;
                        best_idx = Some(i);
                    }
                }
            }

            if let Some(idx) = best_idx {
                let p = &PROCS.pool[idx];
                let mut inner = p.inner.lock();

                if inner.state == ProcState::RUNNABLE {
                    let round = ROUND.load(Ordering::Relaxed);
                    let (_, wait) = Self::features(idx, round, round.max(1));
                    self.epoch_dispatches = self.epoch_dispatches.saturating_add(1);
                    self.epoch_max_wait = self.epoch_max_wait.max(wait);
                    self.epoch_dispatch_mask |= 1u64 << idx;

                    inner.state = ProcState::RUNNING;
                    unsafe {
                        (*c).proc.replace(Arc::clone(p));
                        swtch(&mut (*c).context, &p.data().context);
                        (*c).proc.take();
                    }

                    let new_round = ROUND.fetch_add(1, Ordering::AcqRel).saturating_add(1);
                    LAST_RUN[idx].store(new_round, Ordering::Release);
                    RUN_COUNT[idx].fetch_add(1, Ordering::AcqRel);

                    if self.epoch_dispatches >= EPOCH_LEN {
                        self.learn();
                    }
                    self.scan_offset = (idx + 1) % NPROC;
                }
            }
        }
    }
}
