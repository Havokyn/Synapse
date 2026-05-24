use crate::scheduler::TickSample;

#[must_use]
pub fn p99_jitter_us(samples: &[TickSample]) -> u64 {
    if samples.is_empty() {
        return 0;
    }
    let mut values = samples
        .iter()
        .map(|sample| sample.jitter_us)
        .collect::<Vec<_>>();
    values.sort_unstable();
    let index = ((values.len() * 99).div_ceil(100)).saturating_sub(1);
    values[index]
}
