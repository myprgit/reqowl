//! Hides secret values (from `env/*.local.toml`) in anything that is shown
//! or stored: CLI output, history, pinned responses.

use crate::vars::Context;

/// Values shorter than this are not masked: replacing every `1` or `ok`
/// would garble output without protecting anything.
const MIN_LEN: usize = 4;

#[derive(Debug, Clone, Default)]
pub struct Masker {
    /// (value, name), longest value first so overlapping secrets mask whole.
    secrets: Vec<(String, String)>,
}

impl Masker {
    pub fn new(ctx: &Context) -> Self {
        Self::from_pairs(ctx.secret_values())
    }

    pub fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut secrets: Vec<(String, String)> = pairs
            .into_iter()
            .filter(|(_, v)| v.len() >= MIN_LEN)
            .map(|(k, v)| (v.to_string(), k.to_string()))
            .collect();
        secrets.sort_by_key(|s| std::cmp::Reverse(s.0.len()));
        Masker { secrets }
    }

    pub fn is_empty(&self) -> bool {
        self.secrets.is_empty()
    }

    pub fn mask(&self, text: &str) -> String {
        let mut out = text.to_string();
        for (value, name) in &self.secrets {
            if out.contains(value.as_str()) {
                out = out.replace(value.as_str(), &format!("‹secret:{name}›"));
            }
            // Secrets often travel percent-encoded in URLs and forms.
            let enc = percent_encode(value);
            if enc != *value && out.contains(&enc) {
                out = out.replace(&enc, &format!("‹secret:{name}›"));
            }
        }
        out
    }
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_longest_first_and_encoded() {
        let m = Masker::from_pairs([
            ("short", "abcd"),
            ("long", "abcdef"),
            ("tiny", "x"),
            ("pw", "p@ss word"),
        ]);
        assert_eq!(m.mask("abcdef abcd x"), "‹secret:long› ‹secret:short› x");
        assert_eq!(m.mask("?pw=p%40ss%20word"), "?pw=‹secret:pw›");
        assert!(Masker::from_pairs([]).is_empty());
    }
}
