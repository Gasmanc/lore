//! Token counting using the `cl100k_base` BPE tokenizer (the same vocabulary
//! used by GPT-4 and `text-embedding-ada-002`).
//!
//! Token counts drive chunk-size decisions throughout the build pipeline.

use lore_core::LoreError;
use tiktoken_rs::CoreBPE;

/// Counts tokens using the `cl100k_base` BPE tokenizer.
pub struct TokenCounter {
    bpe: CoreBPE,
}

impl TokenCounter {
    /// Initialise a `TokenCounter` with the `cl100k_base` vocabulary.
    ///
    /// # Errors
    ///
    /// Returns [`LoreError::Parse`] if the tokenizer data cannot be loaded.
    pub fn new() -> Result<Self, LoreError> {
        let bpe = tiktoken_rs::cl100k_base()
            .map_err(|e| LoreError::Parse(format!("failed to initialise tokenizer: {e}")))?;
        Ok(Self { bpe })
    }

    /// Count the number of `cl100k_base` tokens in `text`.
    ///
    /// The count is capped at [`u32::MAX`], which no realistic document can reach.
    #[must_use]
    pub fn count(&self, text: &str) -> u32 {
        // Encode with special tokens to match real-world usage in embedding
        // models that use the same vocabulary.
        let n = self.bpe.encode_with_special_tokens(text).len();
        // A document would need > 4 billion tokens to overflow u32.
        #[allow(clippy::cast_possible_truncation)]
        let n32 = n as u32;
        n32
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn counter() -> TokenCounter {
        TokenCounter::new().expect("cl100k_base must initialise")
    }

    #[test]
    fn test_token_count_prose() {
        let text = "The quick brown fox jumps over the lazy dog. \
            Rust is a systems programming language focused on three goals: \
            safety, speed, and concurrency. It accomplishes these goals \
            without a garbage collector, making it useful for a number of \
            use cases other languages aren't good at: embedding in other \
            languages, programs with specific space and time requirements, \
            and writing low-level code, like device drivers and operating systems.";
        let count = counter().count(text);
        // Rough sanity check: a 100-word paragraph should be 80–130 tokens.
        assert!(
            (80..=130).contains(&count),
            "expected 80–130 tokens for prose, got {count}"
        );
    }

    #[test]
    fn test_token_count_code() {
        let code = r"pub fn fibonacci(n: u64) -> u64 {
    match n {
        0 => 0,
        1 => 1,
        _ => fibonacci(n - 1) + fibonacci(n - 2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fib() {
        assert_eq!(fibonacci(10), 55);
    }
}";
        let count = counter().count(code);
        // Code tokenises more finely than prose: more tokens per character.
        // Assert count > len / 6 (i.e. average of at least 1 token per 6 chars).
        assert!(
            count as usize > code.len() / 6,
            "expected code to tokenise densely, got {count} tokens for {} chars",
            code.len()
        );
    }

    #[test]
    fn test_empty_string() {
        assert_eq!(counter().count(""), 0);
    }
}
