//! A page is a Markdown file. Runnable requests are fenced code blocks
//! whose info string starts with `http`. Everything else is free-form notes.

use std::collections::BTreeMap;
use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// Info-string languages treated as request blocks.
pub const REQUEST_LANGS: &[&str] = &["http"];

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// Position among the page's request blocks, starting at 0.
    pub index: usize,
    /// Full info string after the opening fence, e.g. `http` or `http title`.
    pub info: String,
    /// Code between the fences.
    pub content: String,
    /// Byte range of the whole block, fences included.
    pub range: Range<usize>,
    /// 1-based line of the first content line (the line after the opening fence).
    pub content_line: usize,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Page {
    /// Flat `key: value` pairs from a leading `---` front-matter block.
    pub front_matter: BTreeMap<String, String>,
    pub blocks: Vec<Block>,
}

pub fn parse(text: &str) -> Page {
    let front_matter = parse_front_matter(text);
    let mut blocks = Vec::new();

    let parser = Parser::new_ext(text, Options::ENABLE_YAML_STYLE_METADATA_BLOCKS);
    let mut current: Option<(String, Range<usize>, String)> = None;

    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                let lang = info.split_whitespace().next().unwrap_or("");
                if REQUEST_LANGS.iter().any(|l| l.eq_ignore_ascii_case(lang)) {
                    current = Some((info.to_string(), range, String::new()));
                }
            }
            Event::Text(t) => {
                if let Some((_, _, content)) = current.as_mut() {
                    content.push_str(&t);
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((info, range, content)) = current.take() {
                    let content_line = line_of(text, range.start) + 1;
                    blocks.push(Block {
                        index: blocks.len(),
                        info,
                        content,
                        range,
                        content_line,
                    });
                }
            }
            _ => {}
        }
    }

    Page { front_matter, blocks }
}

/// 1-based line number of a byte offset.
fn line_of(text: &str, offset: usize) -> usize {
    text[..offset].bytes().filter(|&b| b == b'\n').count() + 1
}

/// Reads a leading `---` block of flat `key: value` lines. Nested YAML is
/// ignored; front matter only carries page variables.
fn parse_front_matter(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return out;
    }
    for line in lines {
        let line = line.trim_end();
        if line == "---" || line == "..." {
            return out;
        }
        if line.starts_with([' ', '\t', '#']) {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            let v = v.trim();
            let v = v
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
                .unwrap_or(v);
            out.insert(k.trim().to_string(), v.to_string());
        }
    }
    // No closing fence: not front matter.
    BTreeMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "---\nbase: http://localhost:8080\ntitle: \"Auth\"\n---\n\
# Login\n\nSome notes.\n\n```http\n# @name login\nPOST {{base}}/login\n\n{\"a\":1}\n```\n\n\
```json\n{\"not\": \"a request\"}\n```\n\n~~~http\nGET {{base}}/me\n~~~\n";

    #[test]
    fn finds_http_blocks_only() {
        let page = parse(PAGE);
        assert_eq!(page.blocks.len(), 2);
        assert_eq!(
            page.blocks[0].content,
            "# @name login\nPOST {{base}}/login\n\n{\"a\":1}\n"
        );
        assert_eq!(page.blocks[1].content, "GET {{base}}/me\n");
        assert_eq!(page.blocks[1].index, 1);
    }

    #[test]
    fn records_lines_and_ranges() {
        let page = parse(PAGE);
        let b = &page.blocks[0];
        assert_eq!(b.content_line, 10);
        assert!(PAGE[b.range.clone()].starts_with("```http"));
        assert!(PAGE[b.range.clone()].trim_end().ends_with("```"));
    }

    #[test]
    fn front_matter() {
        let page = parse(PAGE);
        assert_eq!(page.front_matter["base"], "http://localhost:8080");
        assert_eq!(page.front_matter["title"], "Auth");
    }

    #[test]
    fn unclosed_front_matter_is_ignored() {
        assert!(parse("---\na: b\n").front_matter.is_empty());
    }
}
