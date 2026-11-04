//! Cataloged MLS §12.9 external C functions and their reference semantics.
//!
//! An external body is foreign code, so a backend can only run one whose
//! meaning every consumer agrees on. This catalog names the external C
//! functions Rumoca executes: the native evaluators run the bit-exact
//! transcription below and packaged C targets call the C symbol itself,
//! linked from the MSL `ModelicaExternalC` sources they ship. Anything else
//! stays refused with its name.
//!
//! The three MSL `Modelica.Math.Random.Generators.*.random` generators share
//! one interface, `random(Integer stateIn[n]) -> (Real result, Integer
//! stateOut[n])`, implemented by `ModelicaRandom_<generator>(stateIn,
//! stateOut, result)` in `ModelicaRandom.c` (MSL 4.x). The C functions read
//! the Modelica `Integer` state as `int32_t` words of little-endian
//! `uint64_t` lanes; the same reinterpretation is spelled out here.

use serde::{Deserialize, Serialize};

/// One external C function of the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelicaExternalFunction {
    Xorshift64star,
    Xorshift128plus,
    Xorshift1024star,
}

impl ModelicaExternalFunction {
    /// The catalog entry a C external symbol names.
    #[must_use]
    pub fn from_c_symbol(symbol: &str) -> Option<Self> {
        match symbol {
            "ModelicaRandom_xorshift64star" => Some(Self::Xorshift64star),
            "ModelicaRandom_xorshift128plus" => Some(Self::Xorshift128plus),
            "ModelicaRandom_xorshift1024star" => Some(Self::Xorshift1024star),
            _ => None,
        }
    }

    /// The C symbol this entry calls.
    #[must_use]
    pub const fn c_symbol(self) -> &'static str {
        match self {
            Self::Xorshift64star => "ModelicaRandom_xorshift64star",
            Self::Xorshift128plus => "ModelicaRandom_xorshift128plus",
            Self::Xorshift1024star => "ModelicaRandom_xorshift1024star",
        }
    }

    /// The MSL `ModelicaExternalC` source file that defines the symbol.
    #[must_use]
    pub const fn c_source(self) -> &'static str {
        "ModelicaRandom.c"
    }

    /// The length of the generator's Integer state vector.
    #[must_use]
    pub const fn state_len(self) -> u32 {
        match self {
            Self::Xorshift64star => 2,
            Self::Xorshift128plus => 4,
            Self::Xorshift1024star => 33,
        }
    }

    /// One generator step: `(result, stateOut) = f(stateIn)`.
    ///
    /// `state` must have [`Self::state_len`] elements; each is converted to
    /// C `int` the way the C call converts it (two's-complement truncation).
    #[must_use]
    pub fn step(self, state: &[i64]) -> (f64, Vec<i64>) {
        let words = state.iter().map(|&value| value as i32).collect::<Vec<_>>();
        let (result, words) = match self {
            Self::Xorshift64star => xorshift64star(&words),
            Self::Xorshift128plus => xorshift128plus(&words),
            Self::Xorshift1024star => xorshift1024star(&words),
        };
        (result, words.into_iter().map(i64::from).collect())
    }
}

/// The elementary builtin an external `"builtin"` or `"C"` symbol names when
/// the C math library function of that name has the builtin's
/// `double(double[, double])` signature (`sin`, `atan2`, `exp`, ...).
///
/// The MSL declares `Modelica.Math.sin` and its siblings as `external
/// "builtin"` (MLS §12.9.1); a call of such a function over scalar Reals is
/// that builtin.
#[must_use]
pub fn elementary_math_builtin(symbol: &str) -> Option<crate::BuiltinFunction> {
    use crate::BuiltinFunction as B;
    let builtin = B::from_name(symbol)?;
    matches!(
        builtin,
        B::Sin
            | B::Cos
            | B::Tan
            | B::Asin
            | B::Acos
            | B::Atan
            | B::Atan2
            | B::Sinh
            | B::Cosh
            | B::Tanh
            | B::Exp
            | B::Log
            | B::Log10
            | B::Sqrt
    )
    .then_some(builtin)
}

/// `2^-64`, `ModelicaRandom_INVM64`.
const INVM64: f64 = 5.421_010_862_427_522e-20;

/// `ModelicaRandom_RAND`: a 64-bit draw as a double in `(0, 1]`.
fn unit(draw: u64) -> f64 {
    (draw as i64) as f64 * INVM64 + 0.5
}

fn lane(words: &[i32], index: usize) -> u64 {
    u64::from(words[2 * index] as u32) | (u64::from(words[2 * index + 1] as u32) << 32)
}

fn push_lane(words: &mut Vec<i32>, value: u64) {
    words.push(value as u32 as i32);
    words.push((value >> 32) as u32 as i32);
}

fn xorshift64star(state: &[i32]) -> (f64, Vec<i32>) {
    let mut x = lane(state, 0);
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    x = x.wrapping_mul(2_685_821_657_736_338_717);
    let mut out = Vec::with_capacity(2);
    push_lane(&mut out, x);
    (unit(x), out)
}

fn xorshift128plus(state: &[i32]) -> (f64, Vec<i32>) {
    let mut s1 = lane(state, 0);
    let s0 = lane(state, 1);
    s1 ^= s1 << 23;
    let next = (s1 ^ s0 ^ (s1 >> 17) ^ (s0 >> 26)).wrapping_add(s0);
    let mut out = Vec::with_capacity(4);
    push_lane(&mut out, s0);
    push_lane(&mut out, next);
    (unit(next), out)
}

fn xorshift1024star(state: &[i32]) -> (f64, Vec<i32>) {
    let mut lanes = (0..16).map(|index| lane(state, index)).collect::<Vec<_>>();
    let mut p = (state[32] & 15) as usize;
    let mut s0 = lanes[p];
    p = (p + 1) & 15;
    let mut s1 = lanes[p];
    s1 ^= s1 << 31;
    s1 ^= s1 >> 11;
    s0 ^= s0 >> 30;
    lanes[p] = s0 ^ s1;
    let result = unit(lanes[p].wrapping_mul(1_181_783_497_276_652_981));
    let mut out = Vec::with_capacity(33);
    for value in lanes {
        push_lane(&mut out, value);
    }
    out.push(p as i32);
    (result, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(
        function: ModelicaExternalFunction,
        mut state: Vec<i64>,
        steps: usize,
    ) -> (f64, Vec<i64>) {
        let mut draw = 0.0;
        for _ in 0..steps {
            (draw, state) = function.step(&state);
        }
        (draw, state)
    }

    // Reference values below were produced by compiling the MSL 4.x
    // `ModelicaRandom.c` with gcc and iterating each generator from the
    // stated seed state.

    #[test]
    fn xorshift64star_matches_the_msl_c_source() {
        let (draw, state) = run(ModelicaExternalFunction::Xorshift64star, vec![1, 2], 10);
        assert_eq!(state, [2_049_721_705, -416_146_649]);
        assert_eq!(draw.to_bits(), 0.403_108_307_970_044_65_f64.to_bits());
    }

    #[test]
    fn xorshift128plus_matches_the_msl_c_source() {
        let seed = vec![123_456_789, -987_654_321, 42, -7];
        let (draw, state) = run(ModelicaExternalFunction::Xorshift128plus, seed, 5);
        assert_eq!(
            state,
            [-1_231_322_253, 2_120_282_675, 970_281_378, 582_281_864]
        );
        assert_eq!(draw.to_bits(), 0.635_573_061_235_694_f64.to_bits());
    }

    #[test]
    fn xorshift1024star_matches_the_msl_c_source() {
        let mut seed = (0..32u32)
            .map(|k| i64::from(k.wrapping_mul(2_654_435_761) as i32))
            .collect::<Vec<_>>();
        seed.push(5);
        let (draw, state) = run(ModelicaExternalFunction::Xorshift1024star, seed, 20);
        assert_eq!(state, XORSHIFT1024STAR_AFTER_TWENTY);
        assert_eq!(draw.to_bits(), 0.157_525_450_962_294_6_f64.to_bits());
    }

    const XORSHIFT1024STAR_AFTER_TWENTY: [i64; 33] = [
        -1_156_005_442,
        245_284_451,
        692_799_116,
        1_241_987_299,
        459_626_616,
        -513_162_105,
        -216_516_135,
        1_750_785_543,
        -1_669_940_425,
        528_128_559,
        -1_837_902_277,
        1_141_288_490,
        206_465_460,
        -221_024_693,
        -323_003_066,
        1_242_504_071,
        590_728_858,
        696_946_381,
        1_952_989_384,
        -1_411_190_773,
        -470_981_192,
        -228_339_152,
        1_026_900_163,
        149_247_349,
        187_292_642,
        -1_861_927_654,
        1_415_397_435,
        -1_240_914_863,
        369_187_851,
        -46_128_387,
        -812_595_895,
        279_576_893,
        9,
    ];
}
