//! Pure-Rust Data-Oblivious TurboQuant Vector Quantization Engine
//! (inspired by Google Research / ICLR 2026 TurboQuant, `turbovec`, and `turboquant-pytorch`).
//!
//! Pipeline (zero training, zero external ML deps, zero regex, zero `.unwrap()`):
//! 1. L2-normalize input vector onto the unit hypersphere (storing `norm: f32`).
//! 2. Apply randomized orthogonal rotation via deterministic Rademacher sign flips +
//!    in-place Fast Walsh-Hadamard Transform (`O(d log d)` butterfly arithmetic) so
//!    coordinates distribute as universal `N(0, 1/d)`.
//! 3. Quantize each rotated coordinate via `const` Gaussian Lloyd-Max thresholds & centroids
//!    (`partition_point` lookup, zero `if/else` chains) into packed 4-bit (8x smaller) or
//!    2-bit (16x smaller) integer codes.
//! 4. Optional 1-bit QJL (Quantized Johnson-Lindenstrauss) residual sign sketch for
//!    mathematically unbiased inner-product estimation.
//! 5. Asymmetric compressed scoring directly on packed bytes without decompressing `f32` slices.

use crate::algorithms::{is_stopword, tokenize_terms};
use zenko_core::{
    TURBOQUANT_BIGRAM_WEIGHT, TURBOQUANT_CHAR_NGRAM_MAX, TURBOQUANT_CHAR_NGRAM_MIN,
    TURBOQUANT_CHAR_NGRAM_WEIGHT, TURBOQUANT_DEFAULT_DIM, TURBOQUANT_FLAG_QJL,
    TURBOQUANT_LLOYD_MAX_2BIT_CENTROIDS, TURBOQUANT_LLOYD_MAX_2BIT_THRESHOLDS,
    TURBOQUANT_LLOYD_MAX_4BIT_CENTROIDS, TURBOQUANT_LLOYD_MAX_4BIT_THRESHOLDS, TURBOQUANT_MAGIC,
    TURBOQUANT_MAX_DIM, TURBOQUANT_MIN_DIM, TURBOQUANT_SEED_MAIN, TURBOQUANT_SEED_QJL,
    TURBOQUANT_TOKEN_WEIGHT, TurboQuantBits,
};

#[inline]
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[inline]
fn fnv1a_64_tagged(tag: u8, bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325 ^ (tag as u64);
    hash = hash.wrapping_mul(0x0100_0000_01b3);
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// Normalizes any requested dimension up to the nearest power-of-two within
/// `[TURBOQUANT_MIN_DIM, TURBOQUANT_MAX_DIM]`.
#[inline]
pub fn normalize_turboquant_dim(dim: usize) -> usize {
    dim.clamp(TURBOQUANT_MIN_DIM, TURBOQUANT_MAX_DIM)
        .next_power_of_two()
        .min(TURBOQUANT_MAX_DIM)
}

fn build_rademacher_signs(dim: usize, seed: u64) -> Vec<f32> {
    let mut state = seed ^ (dim as u64).wrapping_mul(0x517C_C1B7_2722_0A95);
    let mut signs = Vec::with_capacity(dim);
    let mut word = 0u64;
    for i in 0..dim {
        if (i & 63) == 0 {
            word = splitmix64(&mut state);
        }
        let s = match (word >> (i & 63)) & 1 == 1 {
            true => 1.0f32,
            false => -1.0f32,
        };
        signs.push(s);
    }
    signs
}

/// Orthonormal in-place Fast Walsh-Hadamard Transform (`O(d log d)`).
/// Requires `buf.len()` to be a non-zero power of two.
pub fn fwht_inplace(buf: &mut [f32]) {
    let len = buf.len();
    if len <= 1 || !len.is_power_of_two() {
        return;
    }
    let mut h = 1usize;
    while h < len {
        let step = h << 1;
        let mut i = 0usize;
        while i < len {
            for j in i..(i + h) {
                let x = buf[j];
                let y = buf[j + h];
                buf[j] = x + y;
                buf[j + h] = x - y;
            }
            i += step;
        }
        h = step;
    }
    let inv_sqrt = 1.0f32 / (len as f32).sqrt();
    for v in buf.iter_mut() {
        *v *= inv_sqrt;
    }
}

#[inline]
fn l2_norm(v: &[f32]) -> f32 {
    v.iter().map(|&x| x * x).sum::<f32>().sqrt()
}

/// Compact bit-packed TurboQuant vector suitable for SQLite BLOB storage and direct
/// compressed asymmetric inner-product scoring.
#[derive(Debug, Clone, PartialEq)]
pub struct TurboQuantVector {
    pub dim: u16,
    pub bits: TurboQuantBits,
    pub norm: f32,
    pub packed_codes: Vec<u8>,
    pub qjl_norm: f32,
    pub qjl_signs: Vec<u8>,
}

impl TurboQuantVector {
    pub fn has_qjl(&self) -> bool {
        !self.qjl_signs.is_empty()
    }

    pub fn byte_len(&self) -> usize {
        self.packed_codes.len() + self.qjl_signs.len() + 12 + usize::from(self.has_qjl()) * 4
    }

    /// Deterministic little-endian binary serialization (`ZTQ1` header + packed codes + optional QJL).
    pub fn to_bytes(&self) -> Vec<u8> {
        let has_qjl = self.has_qjl();
        let flags = match has_qjl {
            true => TURBOQUANT_FLAG_QJL,
            false => 0u8,
        };
        let mut out = Vec::with_capacity(self.byte_len());
        out.extend_from_slice(&TURBOQUANT_MAGIC);
        out.extend_from_slice(&self.dim.to_le_bytes());
        out.push(self.bits.bits_per_coord());
        out.push(flags);
        out.extend_from_slice(&self.norm.to_le_bytes());
        if has_qjl {
            out.extend_from_slice(&self.qjl_norm.to_le_bytes());
            out.extend_from_slice(&self.qjl_signs);
        }
        out.extend_from_slice(&self.packed_codes);
        out
    }

    /// Strict zero-panic binary deserialization validating magic, power-of-two dim, and byte lengths.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 12 || bytes[..4] != TURBOQUANT_MAGIC {
            return None;
        }
        let dim = u16::from_le_bytes([bytes[4], bytes[5]]);
        let dim_usize = dim as usize;
        if !(TURBOQUANT_MIN_DIM..=TURBOQUANT_MAX_DIM).contains(&dim_usize)
            || !dim_usize.is_power_of_two()
        {
            return None;
        }
        let bits = TurboQuantBits::from_u8(bytes[6])?;
        let flags = bytes[7];
        let norm = f32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        if !norm.is_finite() || norm < 0.0 {
            return None;
        }
        let has_qjl = (flags & TURBOQUANT_FLAG_QJL) != 0;
        let expected_codes = dim_usize / bits.coords_per_byte();
        let expected_qjl = match has_qjl {
            true => dim_usize / 8,
            false => 0,
        };
        let mut cursor = 12usize;
        let (qjl_norm, qjl_signs) = match has_qjl {
            true => {
                if bytes.len() < cursor + 4 + expected_qjl + expected_codes {
                    return None;
                }
                let qn = f32::from_le_bytes([
                    bytes[cursor],
                    bytes[cursor + 1],
                    bytes[cursor + 2],
                    bytes[cursor + 3],
                ]);
                cursor += 4;
                if !qn.is_finite() || qn < 0.0 {
                    return None;
                }
                let qs = bytes[cursor..cursor + expected_qjl].to_vec();
                cursor += expected_qjl;
                (qn, qs)
            }
            false => {
                if bytes.len() < cursor + expected_codes {
                    return None;
                }
                (0.0, Vec::new())
            }
        };
        if bytes.len() != cursor + expected_codes {
            return None;
        }
        let packed_codes = bytes[cursor..cursor + expected_codes].to_vec();
        Some(Self {
            dim,
            bits,
            norm,
            packed_codes,
            qjl_norm,
            qjl_signs,
        })
    }
}

/// Pre-rotated query vector enabling zero-allocation asymmetric dot-product & cosine scoring
/// across arbitrary batches of compressed `TurboQuantVector`s.
#[derive(Debug, Clone)]
pub struct PreparedTurboQuery {
    dim: usize,
    norm: f32,
    rotated_scaled: Vec<f32>,
    qjl_rotated: Option<Vec<f32>>,
}

impl PreparedTurboQuery {
    pub fn dim(&self) -> usize {
        self.dim
    }

    pub fn is_zero(&self) -> bool {
        self.norm <= f32::EPSILON
    }

    /// Estimates cosine similarity `cos(query, doc)` in `[-1.0, 1.0]` directly from packed bytes.
    pub fn cosine(&self, doc: &TurboQuantVector) -> f32 {
        if self.is_zero() || doc.norm <= f32::EPSILON || (doc.dim as usize) != self.dim {
            return 0.0;
        }
        let base_dot = match doc.bits {
            TurboQuantBits::Bits4 => dot_4bit(&self.rotated_scaled, &doc.packed_codes),
            TurboQuantBits::Bits2 => dot_2bit(&self.rotated_scaled, &doc.packed_codes),
        };
        let qjl_corr = match (&self.qjl_rotated, doc.has_qjl()) {
            (Some(q_qjl), true) if doc.qjl_norm > f32::EPSILON => {
                let signed_sum = dot_1bit_signs(q_qjl, &doc.qjl_signs);
                let factor = (std::f32::consts::FRAC_PI_2 / (self.dim as f32)).sqrt();
                factor * doc.qjl_norm * signed_sum
            }
            _ => 0.0,
        };
        (base_dot + qjl_corr).clamp(-1.0, 1.0)
    }

    /// Estimates raw inner product `<query, doc>` using stored vector norms.
    pub fn inner_product(&self, doc: &TurboQuantVector) -> f32 {
        self.cosine(doc) * self.norm * doc.norm
    }
}

#[inline]
fn dot_4bit(q_scaled: &[f32], packed: &[u8]) -> f32 {
    let mut acc = 0.0f32;
    for (pair_idx, &byte) in packed.iter().enumerate() {
        let base = pair_idx << 1;
        let c0 = (byte & 0x0F) as usize;
        let c1 = (byte >> 4) as usize;
        acc += q_scaled[base] * TURBOQUANT_LLOYD_MAX_4BIT_CENTROIDS[c0]
            + q_scaled[base + 1] * TURBOQUANT_LLOYD_MAX_4BIT_CENTROIDS[c1];
    }
    acc
}

#[inline]
fn dot_2bit(q_scaled: &[f32], packed: &[u8]) -> f32 {
    let mut acc = 0.0f32;
    for (quad_idx, &byte) in packed.iter().enumerate() {
        let base = quad_idx << 2;
        let c0 = (byte & 0x03) as usize;
        let c1 = ((byte >> 2) & 0x03) as usize;
        let c2 = ((byte >> 4) & 0x03) as usize;
        let c3 = ((byte >> 6) & 0x03) as usize;
        acc += q_scaled[base] * TURBOQUANT_LLOYD_MAX_2BIT_CENTROIDS[c0]
            + q_scaled[base + 1] * TURBOQUANT_LLOYD_MAX_2BIT_CENTROIDS[c1]
            + q_scaled[base + 2] * TURBOQUANT_LLOYD_MAX_2BIT_CENTROIDS[c2]
            + q_scaled[base + 3] * TURBOQUANT_LLOYD_MAX_2BIT_CENTROIDS[c3];
    }
    acc
}

#[inline]
fn dot_1bit_signs(q_qjl: &[f32], sign_bytes: &[u8]) -> f32 {
    let mut acc = 0.0f32;
    for (byte_idx, &byte) in sign_bytes.iter().enumerate() {
        let base = byte_idx << 3;
        for bit in 0..8usize {
            let val = q_qjl[base + bit];
            acc += match (byte >> bit) & 1 == 1 {
                true => val,
                false => -val,
            };
        }
    }
    acc
}

/// Deterministic, zero-training TurboQuant codec (FWHT + Lloyd-Max + optional 1-bit QJL).
#[derive(Debug, Clone)]
pub struct TurboQuantCodec {
    dim: usize,
    bits: TurboQuantBits,
    with_qjl: bool,
    signs_main: Vec<f32>,
    signs_qjl: Vec<f32>,
}

impl TurboQuantCodec {
    pub fn new(dim: usize, bits: TurboQuantBits, with_qjl: bool) -> Self {
        let norm_dim = normalize_turboquant_dim(dim);
        let signs_main = build_rademacher_signs(norm_dim, TURBOQUANT_SEED_MAIN);
        let signs_qjl = match with_qjl {
            true => build_rademacher_signs(norm_dim, TURBOQUANT_SEED_QJL),
            false => Vec::new(),
        };
        Self {
            dim: norm_dim,
            bits,
            with_qjl,
            signs_main,
            signs_qjl,
        }
    }

    /// Zenko's default compact 4-bit 256-dim codec (~140 bytes/vector, ~7.3x smaller than `f32`).
    pub fn default_memory_codec() -> Self {
        Self::new(TURBOQUANT_DEFAULT_DIM, TurboQuantBits::Bits4, false)
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    pub fn bits(&self) -> TurboQuantBits {
        self.bits
    }

    /// Projects arbitrary text deterministically into a normalized `dim`-dimensional vector
    /// combining code-aware sub-tokens, adjacent bigrams, and character 3/4-grams (zero LLM/API).
    pub fn text_to_dense(&self, text: &str) -> Vec<f32> {
        let mut vec = vec![0.0f32; self.dim];
        let mask = self.dim - 1;
        let tokens: Vec<String> = tokenize_terms(text)
            .into_iter()
            .filter(|t| !is_stopword(t))
            .collect();
        if tokens.is_empty() {
            return vec;
        }

        let mut add_feature = |tag: u8, bytes: &[u8], weight: f32| {
            let mut st = fnv1a_64_tagged(tag, bytes);
            let h = splitmix64(&mut st);
            let bucket = (h as usize) & mask;
            let sign = match (h >> 63) == 1 {
                true => weight,
                false => -weight,
            };
            vec[bucket] += sign;
        };

        for tok in &tokens {
            let b = tok.as_bytes();
            add_feature(1, b, TURBOQUANT_TOKEN_WEIGHT);
            // Character 3-gram and 4-gram sub-word hashing catches typos, prefixes & morphology
            for n in TURBOQUANT_CHAR_NGRAM_MIN..=TURBOQUANT_CHAR_NGRAM_MAX {
                if b.len() >= n {
                    for win in b.windows(n) {
                        add_feature(2 + (n as u8), win, TURBOQUANT_CHAR_NGRAM_WEIGHT);
                    }
                }
            }
        }
        for pair in tokens.windows(2) {
            let mut buf = Vec::with_capacity(pair[0].len() + pair[1].len() + 1);
            buf.extend_from_slice(pair[0].as_bytes());
            buf.push(b' ');
            buf.extend_from_slice(pair[1].as_bytes());
            add_feature(9, &buf, TURBOQUANT_BIGRAM_WEIGHT);
        }
        vec
    }

    /// Encodes arbitrary text directly into a bit-packed `TurboQuantVector`.
    pub fn encode_text(&self, text: &str) -> TurboQuantVector {
        let dense = self.text_to_dense(text);
        self.encode(&dense)
    }

    /// Prepares a text query once for zero-allocation compressed batch scoring.
    pub fn prepare_text_query(&self, query: &str) -> PreparedTurboQuery {
        let dense = self.text_to_dense(query);
        self.prepare_query(&dense)
    }

    /// Encodes any `f32` vector (automatically zero-padding or truncating to `self.dim`).
    pub fn encode(&self, input: &[f32]) -> TurboQuantVector {
        let mut buf = vec![0.0f32; self.dim];
        let copy_len = input.len().min(self.dim);
        buf[..copy_len].copy_from_slice(&input[..copy_len]);
        let norm = l2_norm(&buf);
        let packed_len = self.dim / self.bits.coords_per_byte();
        if norm <= f32::EPSILON {
            return TurboQuantVector {
                dim: self.dim as u16,
                bits: self.bits,
                norm: 0.0,
                packed_codes: vec![0u8; packed_len],
                qjl_norm: 0.0,
                qjl_signs: match self.with_qjl {
                    true => vec![0u8; self.dim / 8],
                    false => Vec::new(),
                },
            };
        }

        let inv_norm = 1.0f32 / norm;
        for (slot, &sign) in buf.iter_mut().zip(self.signs_main.iter()) {
            *slot = (*slot * inv_norm) * sign;
        }
        fwht_inplace(&mut buf);

        let sqrt_d = (self.dim as f32).sqrt();
        let inv_sqrt_d = 1.0f32 / sqrt_d;
        let mut packed_codes = vec![0u8; packed_len];
        let mut residual = match self.with_qjl {
            true => vec![0.0f32; self.dim],
            false => Vec::new(),
        };

        match self.bits {
            TurboQuantBits::Bits4 => {
                for (i, &val) in buf.iter().enumerate() {
                    let scaled = val * sqrt_d;
                    let code = TURBOQUANT_LLOYD_MAX_4BIT_THRESHOLDS
                        .partition_point(|&t| scaled >= t) as u8;
                    let byte_idx = i >> 1;
                    let shift = (i & 1) << 2;
                    packed_codes[byte_idx] |= (code & 0x0F) << shift;
                    if self.with_qjl {
                        residual[i] =
                            val - TURBOQUANT_LLOYD_MAX_4BIT_CENTROIDS[code as usize] * inv_sqrt_d;
                    }
                }
            }
            TurboQuantBits::Bits2 => {
                for (i, &val) in buf.iter().enumerate() {
                    let scaled = val * sqrt_d;
                    let code = TURBOQUANT_LLOYD_MAX_2BIT_THRESHOLDS
                        .partition_point(|&t| scaled >= t) as u8;
                    let byte_idx = i >> 2;
                    let shift = (i & 3) << 1;
                    packed_codes[byte_idx] |= (code & 0x03) << shift;
                    if self.with_qjl {
                        residual[i] =
                            val - TURBOQUANT_LLOYD_MAX_2BIT_CENTROIDS[code as usize] * inv_sqrt_d;
                    }
                }
            }
        }

        let (qjl_norm, qjl_signs) = match self.with_qjl {
            true => {
                // Transform residual back to original basis, then project through QJL Hadamard.
                fwht_inplace(&mut residual);
                for (r, &s_main) in residual.iter_mut().zip(self.signs_main.iter()) {
                    *r *= s_main;
                }
                let rn = l2_norm(&residual);
                let mut sign_bytes = vec![0u8; self.dim / 8];
                if rn > f32::EPSILON {
                    for (r, &s_qjl) in residual.iter_mut().zip(self.signs_qjl.iter()) {
                        *r *= s_qjl;
                    }
                    fwht_inplace(&mut residual);
                    for (i, &v) in residual.iter().enumerate() {
                        if v >= 0.0 {
                            sign_bytes[i >> 3] |= 1u8 << (i & 7);
                        }
                    }
                }
                (rn, sign_bytes)
            }
            false => (0.0, Vec::new()),
        };

        TurboQuantVector {
            dim: self.dim as u16,
            bits: self.bits,
            norm,
            packed_codes,
            qjl_norm,
            qjl_signs,
        }
    }

    /// Rotates a query vector once so hundreds or thousands of compressed vectors can be scored
    /// in microseconds with zero allocations.
    pub fn prepare_query(&self, query: &[f32]) -> PreparedTurboQuery {
        let mut unit = vec![0.0f32; self.dim];
        let copy_len = query.len().min(self.dim);
        unit[..copy_len].copy_from_slice(&query[..copy_len]);
        let norm = l2_norm(&unit);
        if norm <= f32::EPSILON {
            return PreparedTurboQuery {
                dim: self.dim,
                norm: 0.0,
                rotated_scaled: vec![0.0f32; self.dim],
                qjl_rotated: None,
            };
        }
        let inv_norm = 1.0f32 / norm;
        for v in unit.iter_mut() {
            *v *= inv_norm;
        }

        let mut rotated_scaled: Vec<f32> = unit
            .iter()
            .zip(self.signs_main.iter())
            .map(|(&v, &s)| v * s)
            .collect();
        fwht_inplace(&mut rotated_scaled);
        let inv_sqrt_d = 1.0f32 / (self.dim as f32).sqrt();
        for v in rotated_scaled.iter_mut() {
            *v *= inv_sqrt_d;
        }

        let qjl_rotated = match self.with_qjl {
            true => {
                let mut qj: Vec<f32> = unit
                    .iter()
                    .zip(self.signs_qjl.iter())
                    .map(|(&v, &s)| v * s)
                    .collect();
                fwht_inplace(&mut qj);
                Some(qj)
            }
            false => None,
        };

        PreparedTurboQuery {
            dim: self.dim,
            norm,
            rotated_scaled,
            qjl_rotated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_vec(dim: usize, seed: u64) -> Vec<f32> {
        let mut st = seed;
        (0..dim)
            .map(|_| {
                let u = (splitmix64(&mut st) >> 11) as f64 * (1.0 / ((1u64 << 53) as f64));
                (u * 2.0 - 1.0) as f32
            })
            .collect()
    }

    fn exact_cosine(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let denom = l2_norm(a) * l2_norm(b);
        match denom > f32::EPSILON {
            true => dot / denom,
            false => 0.0,
        }
    }

    #[test]
    fn fwht_is_exact_orthonormal_involution() {
        let orig = synthetic_vec(256, 42);
        let mut v = orig.clone();
        fwht_inplace(&mut v);
        assert!((l2_norm(&orig) - l2_norm(&v)).abs() < 1e-4);
        fwht_inplace(&mut v);
        for (a, b) in orig.iter().zip(v.iter()) {
            assert!((a - b).abs() < 1e-4, "FWHT must be self-inverse");
        }
    }

    #[test]
    fn turboquant_4bit_and_qjl_preserve_cosine_and_compress_tightly() {
        let codec4 = TurboQuantCodec::new(256, TurboQuantBits::Bits4, false);
        let codec4_qjl = TurboQuantCodec::new(256, TurboQuantBits::Bits4, true);
        let codec2 = TurboQuantCodec::new(256, TurboQuantBits::Bits2, true);

        let a = synthetic_vec(256, 1001);
        let mut b = a.clone();
        let noise = synthetic_vec(256, 2002);
        for (x, n) in b.iter_mut().zip(noise.iter()) {
            *x = *x * 0.75 + *n * 0.25;
        }
        let truth = exact_cosine(&a, &b);

        let enc4 = codec4.encode(&b);
        let blob = enc4.to_bytes();
        // Raw f32 = 1024 bytes; 4-bit packed + header = 140 bytes (>7x smaller)
        assert_eq!(blob.len(), 140);
        let roundtrip = TurboQuantVector::from_bytes(&blob).expect("roundtrip ok");
        assert_eq!(roundtrip, enc4);

        let est4 = codec4.prepare_query(&a).cosine(&roundtrip);
        let est4_qjl = codec4_qjl
            .prepare_query(&a)
            .cosine(&codec4_qjl.encode(&b));
        let est2 = codec2.prepare_query(&a).cosine(&codec2.encode(&b));

        assert!(
            (est4 - truth).abs() < 0.03,
            "4-bit cosine error too large: truth={truth}, est4={est4}"
        );
        assert!(
            (est4_qjl - truth).abs() < 0.03,
            "4-bit+QJL cosine error too large: truth={truth}, est4_qjl={est4_qjl}"
        );
        assert!(
            (est2 - truth).abs() < 0.12,
            "2-bit+QJL cosine error too large: truth={truth}, est2={est2}"
        );
    }

    #[test]
    fn turboquant_offline_text_catches_morphological_and_typo_variants() {
        let codec = TurboQuantCodec::default_memory_codec();
        let doc_throttle = codec.encode_text(
            "Exponential backoff throttling prevents 429 rate-limit failures in HTTP client retries",
        );
        let doc_unrelated = codec.encode_text(
            "CSS flexbox grid alignment and dark obsidian vermilion theme palette tokens",
        );

        let q = codec.prepare_text_query("throttled retry backof rate limits");
        let sim_hit = q.cosine(&doc_throttle);
        let sim_miss = q.cosine(&doc_unrelated);
        assert!(
            sim_hit > sim_miss + 0.15,
            "expected morphological match ({sim_hit}) to clearly outrank unrelated doc ({sim_miss})"
        );
    }
}
