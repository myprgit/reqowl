//! Run history in `<workspace>/.reqowl/history.db`. The directory gets its
//! own `.gitignore` so it never ends up in the user's repository.
//!
//! Secret values are masked before anything is written (see [`crate::mask`]).

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::env::STATE_DIR;
use crate::error::{Error, Result};
use crate::exec::Response;
use crate::mask::Masker;
use crate::page::Block;
use crate::request::{self, RequestSpec};

/// Bodies beyond this are stored cut off.
pub const MAX_STORED_BODY: usize = 2 * 1024 * 1024;
/// Runs kept per block; older ones are pruned.
pub const KEEP_PER_BLOCK: i64 = 50;

/// Credentials in auth headers are hidden even when they didn't come from a
/// secret file (e.g. OAuth2 tokens, or a token typed into the page). The
/// scheme stays visible so the entry still shows what kind of auth was used.
fn mask_header(name: &str, value: &str, masker: &Masker) -> String {
    let n = name.to_ascii_lowercase();
    if n == "authorization" || n == "proxy-authorization" {
        return match value.split_once(' ') {
            Some((scheme, _)) => format!("{scheme} ‹hidden›"),
            None => "‹hidden›".into(),
        };
    }
    masker.mask(value)
}

/// Stable identity of a block within its page, so history survives edits:
/// `@name` when present, else position plus method and URL template.
pub fn block_key(block: &Block) -> String {
    match request::parse(&block.content, block.content_line) {
        Ok(spec) => match spec.name() {
            Some(name) => format!("@{name}"),
            None => format!("#{} {} {}", block.index + 1, spec.method, spec.url),
        },
        Err(_) => format!("#{}", block.index + 1),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub id: i64,
    /// Unix milliseconds.
    pub ts: i64,
    pub page: String,
    pub block_key: String,
    pub env: Option<String>,
    pub method: String,
    pub url: String,
    pub request_headers: Vec<(String, String)>,
    pub request_body: Option<String>,
    pub status: Option<u16>,
    pub reason: Option<String>,
    pub response_headers: Vec<(String, String)>,
    pub response_body: Option<String>,
    pub body_size: Option<i64>,
    pub body_truncated: bool,
    pub elapsed_ms: Option<i64>,
    pub error: Option<String>,
}

pub struct History {
    conn: Connection,
    pub path: PathBuf,
}

fn db_err(e: rusqlite::Error) -> Error {
    Error::Config {
        path: PathBuf::from("history.db"),
        msg: e.to_string(),
    }
}

/// Creates `<root>/.reqowl/` with a `.gitignore` ignoring everything in it.
pub fn ensure_state_dir(root: &Path) -> Result<PathBuf> {
    let dir = root.join(STATE_DIR);
    fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    let gi = dir.join(".gitignore");
    if !gi.exists() {
        fs::write(&gi, "*\n").map_err(|e| Error::io(&gi, e))?;
    }
    Ok(dir)
}

impl History {
    /// Opens (or creates) the history of a workspace root.
    pub fn open(root: &Path) -> Result<History> {
        let path = ensure_state_dir(root)?.join("history.db");
        let conn = Connection::open(&path).map_err(db_err)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS runs (
                id INTEGER PRIMARY KEY,
                ts INTEGER NOT NULL,
                page TEXT NOT NULL,
                block_key TEXT NOT NULL,
                env TEXT,
                method TEXT NOT NULL,
                url TEXT NOT NULL,
                req_headers TEXT NOT NULL,
                req_body TEXT,
                status INTEGER,
                reason TEXT,
                resp_headers TEXT NOT NULL,
                resp_body TEXT,
                body_size INTEGER,
                body_truncated INTEGER NOT NULL DEFAULT 0,
                elapsed_ms INTEGER,
                error TEXT
             );
             CREATE INDEX IF NOT EXISTS runs_block ON runs(page, block_key, id);",
        )
        .map_err(db_err)?;
        Ok(History { conn, path })
    }

    /// Records a run. `page` is the page path relative to the workspace root.
    pub fn record(
        &self,
        page: &str,
        block_key: &str,
        env: Option<&str>,
        req: &RequestSpec,
        result: std::result::Result<&Response, &str>,
        masker: &Masker,
    ) -> Result<i64> {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let req_headers: Vec<(String, String)> = req
            .headers
            .iter()
            .map(|h| (h.name.clone(), mask_header(&h.name, &h.value, masker)))
            .collect();
        let req_body = req.body.as_deref().map(|b| masker.mask(b));

        let (status, reason, resp_headers, resp_body, size, truncated, elapsed, error) = match result {
            Ok(r) => {
                let mut text = r.body_text();
                let mut truncated = r.truncated;
                if text.len() > MAX_STORED_BODY {
                    let mut cut = MAX_STORED_BODY;
                    while !text.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    text.truncate(cut);
                    truncated = true;
                }
                let headers: Vec<(String, String)> =
                    r.headers.iter().map(|(k, v)| (k.clone(), masker.mask(v))).collect();
                (
                    Some(r.status),
                    Some(r.reason.clone()),
                    headers,
                    Some(masker.mask(&text)),
                    Some(r.body.len() as i64),
                    truncated,
                    Some(r.elapsed.as_millis() as i64),
                    None,
                )
            }
            Err(e) => (None, None, Vec::new(), None, None, false, None, Some(masker.mask(e))),
        };

        self.conn
            .execute(
                "INSERT INTO runs (ts, page, block_key, env, method, url, req_headers, req_body, status, reason,
                    resp_headers, resp_body, body_size, body_truncated, elapsed_ms, error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                params![
                    ts,
                    page,
                    block_key,
                    env,
                    req.method,
                    masker.mask(&req.url),
                    serde_json::to_string(&req_headers).unwrap_or_default(),
                    req_body,
                    status,
                    reason,
                    serde_json::to_string(&resp_headers).unwrap_or_default(),
                    resp_body,
                    size,
                    truncated,
                    elapsed,
                    error
                ],
            )
            .map_err(db_err)?;
        let id = self.conn.last_insert_rowid();
        self.conn
            .execute(
                "DELETE FROM runs WHERE page = ?1 AND block_key = ?2 AND id NOT IN
                   (SELECT id FROM runs WHERE page = ?1 AND block_key = ?2 ORDER BY id DESC LIMIT ?3)",
                params![page, block_key, KEEP_PER_BLOCK],
            )
            .map_err(db_err)?;
        Ok(id)
    }

    /// Newest first.
    pub fn list(&self, page: &str, block_key: Option<&str>, limit: usize) -> Result<Vec<Entry>> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {COLS} FROM runs WHERE page = ?1 AND (?2 IS NULL OR block_key = ?2) ORDER BY id DESC LIMIT ?3"
            ))
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![page, block_key, limit as i64], row_to_entry)
            .map_err(db_err)?;
        rows.collect::<std::result::Result<_, _>>().map_err(db_err)
    }

    /// The latest run of every block on a page, for showing responses when a page opens.
    pub fn latest_per_block(&self, page: &str) -> Result<Vec<Entry>> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {COLS} FROM runs WHERE id IN (SELECT MAX(id) FROM runs WHERE page = ?1 GROUP BY block_key)"
            ))
            .map_err(db_err)?;
        let rows = stmt.query_map(params![page], row_to_entry).map_err(db_err)?;
        rows.collect::<std::result::Result<_, _>>().map_err(db_err)
    }

    pub fn get(&self, id: i64) -> Result<Option<Entry>> {
        self.conn
            .query_row(
                &format!("SELECT {COLS} FROM runs WHERE id = ?1"),
                params![id],
                row_to_entry,
            )
            .optional()
            .map_err(db_err)
    }

    /// Moves history when a page is renamed or moved.
    pub fn rename_page(&self, from: &str, to: &str) -> Result<()> {
        self.conn
            .execute("UPDATE runs SET page = ?2 WHERE page = ?1", params![from, to])
            .map_err(db_err)?;
        // Children of a renamed folder page.
        self.conn
            .execute(
                "UPDATE runs SET page = ?2 || substr(page, length(?1) + 1) WHERE page LIKE ?3",
                params![
                    format!("{}/", from.trim_end_matches(".md")),
                    format!("{}/", to.trim_end_matches(".md")),
                    format!("{}/%", from.trim_end_matches(".md"))
                ],
            )
            .map_err(db_err)?;
        Ok(())
    }
}

const COLS: &str = "id, ts, page, block_key, env, method, url, req_headers, req_body, status, reason,
    resp_headers, resp_body, body_size, body_truncated, elapsed_ms, error";

fn row_to_entry(r: &rusqlite::Row) -> rusqlite::Result<Entry> {
    let headers = |s: String| serde_json::from_str(&s).unwrap_or_default();
    Ok(Entry {
        id: r.get(0)?,
        ts: r.get(1)?,
        page: r.get(2)?,
        block_key: r.get(3)?,
        env: r.get(4)?,
        method: r.get(5)?,
        url: r.get(6)?,
        request_headers: headers(r.get(7)?),
        request_body: r.get(8)?,
        status: r.get(9)?,
        reason: r.get(10)?,
        response_headers: headers(r.get(11)?),
        response_body: r.get(12)?,
        body_size: r.get(13)?,
        body_truncated: r.get(14)?,
        elapsed_ms: r.get(15)?,
        error: r.get(16)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page;
    use std::time::Duration;

    fn response(body: &str) -> Response {
        Response {
            status: 200,
            reason: "OK".into(),
            version: "HTTP/1.1".into(),
            url: "http://h/".into(),
            headers: vec![("x-token".into(), "s3cret-value".into())],
            body: body.as_bytes().to_vec(),
            truncated: false,
            elapsed: Duration::from_millis(12),
        }
    }

    #[test]
    fn keys() {
        let p =
            page::parse("```http\n# @name login\nPOST http://h\n```\n```http\nGET {{b}}/x\n```\n```http\nnope\n```\n");
        let keys: Vec<String> = p.blocks.iter().map(block_key).collect();
        assert_eq!(keys, ["@login", "#2 GET {{b}}/x", "#3"]);
    }

    #[test]
    fn record_masks_and_prunes() {
        let dir = tempfile::tempdir().unwrap();
        let h = History::open(dir.path()).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join(".reqowl/.gitignore")).unwrap(),
            "*\n"
        );

        let masker = Masker::from_pairs([("token", "s3cret-value")]);
        let mut spec = request::parse("GET http://h/?t=s3cret-value\nAuthorization: Bearer s3cret-value", 1).unwrap();
        spec.body = Some("{\"t\":\"s3cret-value\"}".into());

        for i in 0..(KEEP_PER_BLOCK + 3) {
            h.record(
                "A/B.md",
                "@x",
                Some("dev"),
                &spec,
                Ok(&response(&format!("n{i} s3cret-value"))),
                &masker,
            )
            .unwrap();
        }
        h.record("A/B.md", "@y", None, &spec, Err("refused s3cret-value"), &masker)
            .unwrap();

        let list = h.list("A/B.md", Some("@x"), 1000).unwrap();
        assert_eq!(list.len() as i64, KEEP_PER_BLOCK);
        let e = &list[0];
        assert_eq!(
            e.response_body.as_deref(),
            Some(&*format!("n{} ‹secret:token›", KEEP_PER_BLOCK + 2))
        );
        assert_eq!(e.url, "http://h/?t=‹secret:token›");
        assert_eq!(e.request_headers[0].1, "Bearer ‹hidden›");
        assert_eq!(e.response_headers[0].1, "‹secret:token›");
        assert!(e.request_body.as_deref().unwrap().contains("‹secret:token›"));

        // Check the database and its WAL file.
        let needle = b"s3cret-value";
        for f in fs::read_dir(dir.path().join(".reqowl")).unwrap() {
            let raw = fs::read(f.unwrap().path()).unwrap();
            assert!(
                !raw.windows(needle.len()).any(|w| w == needle),
                "secret written to disk"
            );
        }

        let latest = h.latest_per_block("A/B.md").unwrap();
        assert_eq!(latest.len(), 2);
        let err = latest.iter().find(|e| e.block_key == "@y").unwrap();
        assert_eq!(err.error.as_deref(), Some("refused ‹secret:token›"));
        assert!(h.get(err.id).unwrap().is_some());

        h.rename_page("A/B.md", "A/C.md").unwrap();
        assert!(h.list("A/B.md", None, 10).unwrap().is_empty());
        assert_eq!(h.list("A/C.md", Some("@y"), 10).unwrap().len(), 1);
    }
}
