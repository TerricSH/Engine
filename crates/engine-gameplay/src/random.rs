use serde::{Deserialize, Serialize};

/// Small deterministic random stream shared by gameplay rule modules.
///
/// The generator is intentionally simple and serializable: save games,
/// replays, headless tests, and managed ports can all resume at the exact
/// same point in a rule simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    pub fn state(&self) -> u64 {
        self.state
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    pub fn range_u32(&mut self, upper_exclusive: u32) -> u32 {
        if upper_exclusive == 0 {
            return 0;
        }
        (self.next_u64() % u64::from(upper_exclusive)) as u32
    }

    pub fn roll_basis_points(&mut self) -> u16 {
        self.range_u32(10_000) as u16
    }
}

impl Default for DeterministicRng {
    fn default() -> Self {
        Self::new(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_is_replayable_and_bounded() {
        let mut first = DeterministicRng::new(42);
        let mut second = DeterministicRng::new(42);
        for _ in 0..16 {
            assert_eq!(first.next_u64(), second.next_u64());
            assert!(first.range_u32(7) < 7);
            assert!(second.range_u32(7) < 7);
        }
    }

    #[test]
    fn state_round_trips() {
        let mut random = DeterministicRng::new(9);
        let _ = random.next_u64();
        let json = serde_json::to_string(&random).unwrap();
        assert_eq!(
            serde_json::from_str::<DeterministicRng>(&json).unwrap(),
            random
        );
    }
}
