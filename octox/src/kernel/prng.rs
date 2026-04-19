/// Small deterministic PRNG for kernel heuristics.
///
/// This is not cryptographic. It is intended for low-cost scheduler and
/// policy exploration where reproducibility and no-alloc/no-std operation
/// matter more than statistical strength.
pub struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    #[inline]
    pub const fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 1 } else { seed },
        }
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x.max(1);
        self.state
    }

    #[inline]
    pub fn next_i64_inclusive(&mut self, min: i64, max: i64) -> i64 {
        debug_assert!(min <= max);
        let span = (max - min + 1) as u64;
        (self.next_u64() % span) as i64 + min
    }
}
