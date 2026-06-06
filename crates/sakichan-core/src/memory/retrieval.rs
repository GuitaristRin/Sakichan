use super::store::{Memory, MemoryStore};
use crate::error::Result;

/// Check if a character is CJK (Chinese/Japanese/Korean).
fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{4E00}'..='\u{9FFF}' |
        '\u{3400}'..='\u{4DBF}' |
        '\u{2F800}'..='\u{2FA1F}'
    )
}

/// Extracts simple keywords from user input for memory lookup.
/// Splits on whitespace/punctuation, filters short/common words.
/// For CJK text, also extracts individual characters (excluding stop words)
/// so that queries like "张三" can match memories.
pub fn extract_keywords(input: &str) -> Vec<String> {
    let stop_words: std::collections::HashSet<&str> = [
        "的", "了", "是", "在", "我", "有", "和", "就", "不", "人", "都", "一",
        "一个", "上", "也", "很", "到", "说", "要", "去", "你", "会", "着",
        "没有", "看", "好", "自己", "这", "他", "她", "它", "们",
        "the", "a", "an", "is", "are", "was", "were", "be", "been",
        "being", "have", "has", "had", "do", "does", "did", "will",
        "would", "could", "should", "may", "might", "shall", "can",
        "i", "you", "he", "she", "it", "we", "they", "me", "him",
        "her", "us", "them", "my", "your", "his", "its", "our",
        "their", "this", "that", "these", "those", "and", "or",
        "but", "if", "because", "as", "until", "while", "of", "at",
        "by", "for", "with", "about", "against", "between", "into",
        "through", "during", "before", "after", "above", "below",
        "to", "from", "up", "down", "in", "out", "on", "off", "over",
        "under", "again", "further", "then", "once", "here", "there",
        "when", "where", "why", "how", "all", "each", "every", "both",
        "few", "more", "most", "other", "some", "such", "no", "nor",
        "not", "only", "own", "same", "so", "than", "too", "very",
        "just", "because", "also", "what", "which", "who", "whom",
    ]
    .into_iter()
    .collect();

    let mut keywords: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut cjk_accumulator = String::new();

    for ch in input.chars() {
        if ch.is_alphanumeric() && !is_cjk(ch) {
            // ASCII alphanumeric — accumulate normally
            current.push(ch);
            // Flush CJK accumulator
            if !cjk_accumulator.is_empty() {
                push_cjk_keywords(&cjk_accumulator, &stop_words, &mut keywords);
                cjk_accumulator.clear();
            }
        } else if is_cjk(ch) {
            // CJK character — flush any ASCII word
            if !current.is_empty() {
                if current.len() >= 2 && !stop_words.contains(current.as_str()) {
                    keywords.push(current.clone());
                }
                current.clear();
            }
            cjk_accumulator.push(ch);
        } else {
            // Separator — flush both
            if !current.is_empty() {
                if current.len() >= 2 && !stop_words.contains(current.as_str()) {
                    keywords.push(current.clone());
                }
                current.clear();
            }
            if !cjk_accumulator.is_empty() {
                push_cjk_keywords(&cjk_accumulator, &stop_words, &mut keywords);
                cjk_accumulator.clear();
            }
        }
    }

    // Flush remaining
    if !current.is_empty() && current.len() >= 2 && !stop_words.contains(current.as_str()) {
        keywords.push(current);
    }
    if !cjk_accumulator.is_empty() {
        push_cjk_keywords(&cjk_accumulator, &stop_words, &mut keywords);
    }

    // Deduplicate
    keywords.sort();
    keywords.dedup();
    keywords
}

/// Extract meaningful CJK substrings: bigrams and trigrams that exclude stop words.
fn push_cjk_keywords(
    text: &str,
    stop_words: &std::collections::HashSet<&str>,
    keywords: &mut Vec<String>,
) {
    let chars: Vec<char> = text.chars().collect();

    // Extract bigrams (2-char sequences)
    if chars.len() >= 2 {
        for window in chars.windows(2) {
            let bigram: String = window.iter().collect();
            if !stop_words.contains(bigram.as_str()) {
                keywords.push(bigram);
            }
        }
    }

    // Extract trigrams (3-char sequences)
    if chars.len() >= 3 {
        for window in chars.windows(3) {
            let trigram: String = window.iter().collect();
            if !stop_words.contains(trigram.as_str()) {
                keywords.push(trigram);
            }
        }
    }

    // If very short (1 char), include it if not a stop word
    if chars.len() == 1 {
        let s = text.to_string();
        if !stop_words.contains(s.as_str()) {
            keywords.push(s);
        }
    }
}

/// Retrieves relevant memories for a given user input.
pub struct MemoryRetriever {
    store: MemoryStore,
    top_k: usize,
}

impl MemoryRetriever {
    pub fn new(store: MemoryStore, top_k: usize) -> Self {
        Self { store, top_k }
    }

    /// Retrieve memories relevant to the given input text.
    pub async fn retrieve(&self, input: &str) -> Result<Vec<Memory>> {
        let keywords = extract_keywords(input);
        let mut all_memories: Vec<Memory> = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();

        for kw in &keywords {
            let results = self.store.search(kw, self.top_k)?;
            for mem in results {
                if seen_ids.insert(mem.id) {
                    all_memories.push(mem);
                    if all_memories.len() >= self.top_k {
                        return Ok(all_memories);
                    }
                }
            }
        }

        // If no keyword matches, try a broader approach: return recent memories
        if all_memories.is_empty() {
            all_memories = self.store.list(self.top_k)?;
        }

        Ok(all_memories)
    }

    /// Format memories into a prompt string for injection.
    pub fn format_memories_for_prompt(memories: &[Memory]) -> String {
        if memories.is_empty() {
            return String::new();
        }

        let mut result = String::from("以下是关于用户的长期记忆，请在回答时参考:\n");
        for mem in memories {
            result.push_str(&format!("- {}\n", mem.content));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_keywords() {
        let input = "我的名字是张三，我是一名 Rust 开发者";
        let keywords = extract_keywords(input);
        assert!(keywords.contains(&"张三".to_string()));
        assert!(keywords.contains(&"Rust".to_string()));
        assert!(keywords.contains(&"开发者".to_string()));
        // "我" and "的" should be filtered as stop words
        assert!(!keywords.contains(&"我".to_string()));
        assert!(!keywords.contains(&"的".to_string()));
    }
}