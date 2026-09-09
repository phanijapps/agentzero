//! Shared vector math for embedding similarity and normalization.
//!
//! One implementation of the cosine kernel instead of eleven scattered
//! copies with diverging precision. Both accumulation precisions are
//! provided; pick the one your call site already used — `f64` for store
//! ranking paths (sqlite + engram adapter), `f32` where the value feeds
//! back into `f32` score arithmetic (gateway-memory conflict resolution).

/// Cosine similarity with `f64` accumulation. Returns `0.0` for empty,
/// mismatched-length, or zero-magnitude inputs so search ranks them last
/// rather than panicking.
pub fn cosine_f64(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0f64;
    let mut na = 0f64;
    let mut nb = 0f64;
    for (x, y) in a.iter().zip(b.iter()) {
        let x = *x as f64;
        let y = *y as f64;
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Cosine similarity with `f32` accumulation — use when the result flows
/// back into `f32` score arithmetic and the extra precision would be
/// truncated anyway. Same guards as [`cosine_f64`].
pub fn cosine_f32(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0_f32;
    let mut na = 0.0_f32;
    let mut nb = 0.0_f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

/// Cosine similarity returning `None` for empty, mismatched-length, or
/// zero-magnitude inputs — for pipelines that exclude such rows from
/// results entirely instead of ranking them last.
pub fn cosine_f64_opt(a: &[f32], b: &[f32]) -> Option<f64> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let mut dot = 0f64;
    let mut na = 0f64;
    let mut nb = 0f64;
    for (x, y) in a.iter().zip(b.iter()) {
        let x = *x as f64;
        let y = *y as f64;
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        return None;
    }
    Some(dot / (na.sqrt() * nb.sqrt()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_f64_identical_is_one() {
        let v = vec![1.0_f32, 2.0, 3.0];
        assert!((cosine_f64(&v, &v) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn cosine_f64_orthogonal_is_zero() {
        let a = vec![1.0_f32, 0.0];
        let b = vec![0.0_f32, 1.0];
        assert_eq!(cosine_f64(&a, &b), 0.0);
    }

    #[test]
    fn cosine_f64_mismatched_and_empty_are_zero() {
        assert_eq!(cosine_f64(&[1.0], &[1.0, 2.0]), 0.0);
        assert_eq!(cosine_f64(&[], &[]), 0.0);
        assert_eq!(cosine_f64(&[0.0, 0.0], &[0.0, 0.0]), 0.0);
    }

    #[test]
    fn cosine_f64_opt_returns_none_for_invalid() {
        assert_eq!(cosine_f64_opt(&[1.0], &[1.0, 2.0]), None);
        assert_eq!(cosine_f64_opt(&[], &[]), None);
        assert_eq!(cosine_f64_opt(&[0.0, 0.0], &[0.0, 0.0]), None);
        assert!((cosine_f64_opt(&[1.0, 1.0], &[1.0, 1.0]).unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn cosine_f32_agrees_with_f64_within_tolerance() {
        let a = vec![0.1_f32, 0.2, 0.3, 0.4];
        let b = vec![0.4_f32, 0.3, 0.2, 0.1];
        let diff = (cosine_f32(&a, &b) as f64 - cosine_f64(&a, &b)).abs();
        assert!(diff < 1e-5, "precisions diverged: {diff}");
    }
}
