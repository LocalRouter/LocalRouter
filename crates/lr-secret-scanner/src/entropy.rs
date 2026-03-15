//! Shannon entropy calculator for secret detection

/// Calculate Shannon entropy of a string (bits per character)
///
/// Higher entropy indicates more randomness, which is characteristic of secrets.
/// Typical thresholds:
/// - < 3.0: likely a placeholder or example value
/// - 3.0-3.5: borderline, may be a weak secret
/// - 3.5-4.5: likely a real secret
/// - > 4.5: almost certainly random/secret
pub fn shannon_entropy(s: &str) -> f32 {
    if s.is_empty() {
        return 0.0;
    }

    let mut freq = [0u32; 256];
    let len = s.len() as f32;

    for &b in s.as_bytes() {
        freq[b as usize] += 1;
    }

    freq.iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f32 / len;
            -p * p.log2()
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_string() {
        assert_eq!(shannon_entropy(""), 0.0);
    }

    #[test]
    fn test_single_char() {
        assert_eq!(shannon_entropy("aaaa"), 0.0);
    }

    #[test]
    fn test_low_entropy() {
        // Repeated pattern = low entropy
        let entropy = shannon_entropy("abababababab");
        assert!(entropy < 1.5, "Expected low entropy, got {}", entropy);
    }

    #[test]
    fn test_placeholder_value() {
        // Example/placeholder API key has lower entropy
        let entropy = shannon_entropy("AKIAIOSFODNN7EXAMPLE");
        assert!(
            entropy < 4.0,
            "Expected moderate entropy for placeholder, got {}",
            entropy
        );
    }

    #[test]
    fn test_real_looking_key() {
        // Random-looking string has high entropy
        let entropy = shannon_entropy("aK7mZ9pQ2xR5vN8bW3jL6fT1cY4hD0gS");
        assert!(
            entropy > 3.5,
            "Expected high entropy for random key, got {}",
            entropy
        );
    }

    #[test]
    fn test_hex_string() {
        let entropy = shannon_entropy("a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6");
        assert!(
            entropy > 3.0,
            "Expected decent entropy for hex string, got {}",
            entropy
        );
    }
}
