use synapse_core::{Key, KeyCode, KeystrokeDynamics, KeystrokeNaturalParams};

pub const BIGRAMS: &[&str] = &[
    "th", "he", "in", "er", "an", "re", "on", "at", "en", "nd", "ti", "es", "or", "te", "of", "ed",
    "is", "it", "al", "ar", "st", "to", "nt", "ng", "se", "ha", "as", "ou", "io", "le", "ve", "co",
    "me", "de", "hi", "ri", "ro", "ic", "ne", "ea", "ra", "ce", "li", "ch", "ll", "be", "ma", "si",
    "om", "ur",
];

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash)]
pub struct ModifierMask {
    bits: u8,
}

impl ModifierMask {
    pub const NONE: Self = Self { bits: 0 };
    pub const SHIFT: Self = Self { bits: 1 };
    pub const CTRL: Self = Self { bits: 1 << 1 };
    pub const ALT: Self = Self { bits: 1 << 2 };
    pub const META: Self = Self { bits: 1 << 3 };

    #[must_use]
    pub const fn bits(self) -> u8 {
        self.bits
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.bits & other.bits == other.bits
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.bits == 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct KeystrokeEvent {
    pub r#char: char,
    pub key: Key,
    pub iki_ms_before: u32,
    pub modifier_state: ModifierMask,
}

/// Samples the per-character typing schedule for an action text payload.
///
/// The first event has no preceding inter-keystroke interval, so
/// `iki_ms_before` is always `0` for index `0`.
#[must_use]
pub fn sample_typing_schedule(
    text: &str,
    dynamics: &KeystrokeDynamics,
    seed: Option<u64>,
) -> Vec<KeystrokeEvent> {
    let mut rng = DeterministicRng::new(effective_seed(text, dynamics, seed));
    let mut previous = None;
    let mut events = Vec::with_capacity(text.chars().count());

    for (index, ch) in text.chars().enumerate() {
        let iki_ms_before = if index == 0 {
            0
        } else {
            sample_iki_ms(previous, ch, dynamics, &mut rng)
        };
        let (key, modifier_state) = key_for_char(ch);
        events.push(KeystrokeEvent {
            r#char: ch,
            key,
            iki_ms_before,
            modifier_state,
        });
        previous = Some(ch);
    }

    events
}

fn sample_iki_ms(
    previous: Option<char>,
    current: char,
    dynamics: &KeystrokeDynamics,
    rng: &mut DeterministicRng,
) -> u32 {
    match dynamics {
        KeystrokeDynamics::Burst => 0,
        KeystrokeDynamics::Linear { ms_per_char } => *ms_per_char,
        KeystrokeDynamics::Natural { params } => {
            sample_natural_iki(previous, current, *params, rng)
        }
    }
}

fn sample_natural_iki(
    previous: Option<char>,
    current: char,
    params: KeystrokeNaturalParams,
    rng: &mut DeterministicRng,
) -> u32 {
    let mean = sanitized_non_negative(params.mean_iki_ms);
    if params.bigram_bias && previous.is_some_and(|prev| is_common_bigram(prev, current)) {
        return round_non_negative_to_u32(f64::from(mean) * 0.75);
    }

    let stddev = sanitized_non_negative(params.stddev_ms);
    round_non_negative_to_u32(f64::from(mean) + gaussian(rng, f64::from(stddev)))
}

fn key_for_char(ch: char) -> (Key, ModifierMask) {
    match ch {
        'A'..='Z' => (named_key(ch.to_ascii_lowercase()), ModifierMask::SHIFT),
        'a'..='z' | '0'..='9' => (named_key(ch), ModifierMask::NONE),
        '\n' | '\r' => (named_key_name("enter"), ModifierMask::NONE),
        '\t' => (named_key_name("tab"), ModifierMask::NONE),
        ' ' => (named_key_name("space"), ModifierMask::NONE),
        '!' => (named_key('1'), ModifierMask::SHIFT),
        '@' => (named_key('2'), ModifierMask::SHIFT),
        '#' => (named_key('3'), ModifierMask::SHIFT),
        '$' => (named_key('4'), ModifierMask::SHIFT),
        '%' => (named_key('5'), ModifierMask::SHIFT),
        '^' => (named_key('6'), ModifierMask::SHIFT),
        '&' => (named_key('7'), ModifierMask::SHIFT),
        '*' => (named_key('8'), ModifierMask::SHIFT),
        '(' => (named_key('9'), ModifierMask::SHIFT),
        ')' => (named_key('0'), ModifierMask::SHIFT),
        '_' => (named_key('-'), ModifierMask::SHIFT),
        '+' => (named_key('='), ModifierMask::SHIFT),
        '{' => (named_key('['), ModifierMask::SHIFT),
        '}' => (named_key(']'), ModifierMask::SHIFT),
        '|' => (named_key('\\'), ModifierMask::SHIFT),
        ':' => (named_key(';'), ModifierMask::SHIFT),
        '"' => (named_key('\''), ModifierMask::SHIFT),
        '<' => (named_key(','), ModifierMask::SHIFT),
        '>' => (named_key('.'), ModifierMask::SHIFT),
        '?' => (named_key('/'), ModifierMask::SHIFT),
        '~' => (named_key('`'), ModifierMask::SHIFT),
        '-' | '=' | '[' | ']' | '\\' | ';' | '\'' | ',' | '.' | '/' | '`' => {
            (named_key(ch), ModifierMask::NONE)
        }
        _ => (
            Key {
                code: KeyCode::Symbol { value: ch },
                use_scancode: false,
            },
            ModifierMask::NONE,
        ),
    }
}

fn named_key(value: char) -> Key {
    named_key_name(&value.to_string())
}

fn named_key_name(value: &str) -> Key {
    Key {
        code: KeyCode::Named {
            value: value.to_owned(),
        },
        use_scancode: false,
    }
}

fn is_common_bigram(previous: char, current: char) -> bool {
    let previous = previous.to_ascii_lowercase();
    let current = current.to_ascii_lowercase();
    BIGRAMS.iter().any(|bigram| {
        let mut chars = bigram.chars();
        matches!(
            (chars.next(), chars.next(), chars.next()),
            (Some(left), Some(right), None) if left == previous && right == current
        )
    })
}

const fn sanitized_non_negative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn gaussian(rng: &mut DeterministicRng, stddev: f64) -> f64 {
    if stddev <= 0.0 {
        return 0.0;
    }

    let u1 = rng.next_open_unit();
    let u2 = rng.next_open_unit();
    let z0 = (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos();
    z0 * stddev
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "typing IKIs are rounded and clamped to the public u32 millisecond field for #164"
)]
fn round_non_negative_to_u32(value: f64) -> u32 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    value.round().clamp(0.0, f64::from(u32::MAX)) as u32
}

fn effective_seed(text: &str, dynamics: &KeystrokeDynamics, override_seed: Option<u64>) -> u64 {
    if let Some(seed) = override_seed {
        return seed;
    }

    let mut seed = 0x517c_c1b7_cafe_1640;
    for ch in text.chars() {
        mix_u64(&mut seed, u64::from(u32::from(ch)));
    }

    match dynamics {
        KeystrokeDynamics::Burst => mix_u64(&mut seed, 0),
        KeystrokeDynamics::Linear { ms_per_char } => {
            mix_u64(&mut seed, 1);
            mix_u64(&mut seed, u64::from(*ms_per_char));
        }
        KeystrokeDynamics::Natural { params } => {
            mix_u64(&mut seed, 2);
            mix_u64(&mut seed, u64::from(params.mean_iki_ms.to_bits()));
            mix_u64(&mut seed, u64::from(params.stddev_ms.to_bits()));
            mix_u64(&mut seed, u64::from(params.bigram_bias));
        }
    }

    seed
}

const fn mix_u64(seed: &mut u64, value: u64) {
    *seed ^= value
        .wrapping_add(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(*seed << 6)
        .wrapping_add(*seed >> 2);
}

#[derive(Debug)]
struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    const fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.state;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }

    fn next_open_unit(&mut self) -> f64 {
        (f64::from(self.next_u32()) + 1.0) / (f64::from(u32::MAX) + 2.0)
    }

    fn next_u32(&mut self) -> u32 {
        u32::try_from(self.next_u64() >> 32).unwrap_or(0)
    }
}
