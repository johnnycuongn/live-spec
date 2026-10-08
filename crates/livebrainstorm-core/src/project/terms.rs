//! Tokenizer shared by indexing and queries, so "user profile" in a draft
//! matches `UserProfile`, `user_profile` and `userProfileService`.

const MIN_LEN: usize = 3;

const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "have", "are", "was", "not", "but",
    "you", "your", "will", "can", "use", "using", "into", "when", "then", "than", "what",
    "which", "also", "there", "their", "some", "just", "like", "want", "need", "should",
    "would", "could", "about", "them", "they", "its", "our", "all", "any", "has", "how",
    "get", "set", "let", "var", "const", "function", "return", "import", "export", "pub",
    "mut", "self", "true", "false", "null", "none", "string", "impl", "struct", "class",
    "def", "async", "await", "else", "elif", "while", "new", "type", "maybe", "think",
    "something", "thing", "things", "make", "does", "doing", "done", "here", "where",
];

fn keep(word: &str) -> bool {
    word.len() >= MIN_LEN
        && !word.bytes().all(|b| b.is_ascii_digit())
        && !STOPWORDS.contains(&word)
}

/// Common English and keyword noise; `word` must be lowercase.
pub(crate) fn is_stopword(word: &str) -> bool {
    STOPWORDS.contains(&word)
}

/// Folds plurals so "invoices" in a draft matches `invoice` in code. Applied
/// identically at index and query time; deliberately not a full stemmer, which
/// would mangle identifiers ("caching" -> "cach").
fn singular(word: String) -> String {
    let n = word.len();
    if n > 4 && word.ends_with("ies") {
        format!("{}y", &word[..n - 3])
    } else if n > 4 && ["ses", "xes", "zes", "ches", "shes"].iter().any(|s| word.ends_with(s)) {
        word[..n - 2].to_owned()
    } else if n > 3 && word.ends_with('s') && !word.ends_with("ss") && !word.ends_with("us") {
        word[..n - 1].to_owned()
    } else {
        word
    }
}

/// Lowercased terms: each identifier whole, plus its snake/camel parts.
pub fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for ident in text.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
        if ident.is_empty() {
            continue;
        }
        let whole = ident.to_lowercase();
        let parts = split_parts(ident);
        if parts.len() > 1 && keep(&whole) {
            out.push(singular(whole.replace('_', "")));
        }
        out.extend(parts.into_iter().map(|p| p.to_lowercase()).filter(|p| keep(p)).map(singular));
    }
    out
}

fn split_parts(ident: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    for seg in ident.split('_').filter(|s| !s.is_empty()) {
        let chars: Vec<(usize, char)> = seg.char_indices().collect();
        let mut start = 0;
        for i in 1..chars.len() {
            let (pos, c) = chars[i];
            let prev = chars[i - 1].1;
            let next_lower = chars.get(i + 1).is_some_and(|(_, n)| n.is_lowercase());
            // fooBar | HTTPServer -> HTTP, Server
            if c.is_uppercase() && (prev.is_lowercase() || (prev.is_uppercase() && next_lower)) {
                parts.push(&seg[start..pos]);
                start = pos;
            }
        }
        parts.push(&seg[start..]);
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_identifiers() {
        assert_eq!(tokens("userProfileService"), ["userprofileservice", "user", "profile", "service"]);
        assert_eq!(tokens("HTTPServer"), ["httpserver", "http", "server"]);
        assert_eq!(tokens("auth_token"), ["authtoken", "auth", "token"]);
    }

    #[test]
    fn drops_noise() {
        assert_eq!(tokens("I want to add the 2024 cache"), ["add", "cache"]);
    }

    #[test]
    fn folds_plurals_only() {
        assert_eq!(tokens("invoices categories boxes classes status caching"), ["invoice", "category", "box", "class", "status", "caching"]);
    }
}
