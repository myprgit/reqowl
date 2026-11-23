//! The page tree on disk. A page with id `Auth/Login` is the file
//! `Auth/Login.md`; its child pages live in the folder `Auth/Login/`.
//! A folder may have an `.order` file listing child titles, one per line.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use reqowl_core::env::{ENV_DIR, STATE_DIR};

pub const ORDER_FILE: &str = ".order";

#[derive(Debug, Serialize)]
pub struct Node {
    pub id: String,
    pub title: String,
    /// False for a plain folder without its own `.md`.
    pub has_page: bool,
    pub children: Vec<Node>,
}

pub type Res<T> = Result<T, String>;

fn io<E: std::fmt::Display>(path: &Path) -> impl Fn(E) -> String + '_ {
    move |e| format!("{}: {e}", path.display())
}

/// Rejects ids that could escape the workspace.
pub fn check_id(id: &str) -> Res<()> {
    let segments_ok = id.split('/').all(|seg| !seg.is_empty() && seg != "." && seg != "..");
    let p = Path::new(id);
    if !segments_ok || !p.components().all(|c| matches!(c, Component::Normal(_))) || id.contains('\\') {
        return Err(format!("invalid page id `{id}`"));
    }
    Ok(())
}

pub fn md_path(root: &Path, id: &str) -> PathBuf {
    root.join(format!("{id}.md"))
}

pub fn dir_path(root: &Path, id: &str) -> PathBuf {
    root.join(id)
}

fn parent_id(id: &str) -> Option<&str> {
    id.rsplit_once('/').map(|(p, _)| p)
}

fn title_of(id: &str) -> &str {
    id.rsplit_once('/').map_or(id, |(_, t)| t)
}

fn join_id(parent: Option<&str>, title: &str) -> String {
    match parent {
        Some(p) => format!("{p}/{title}"),
        None => title.to_string(),
    }
}

fn folder(root: &Path, parent: Option<&str>) -> PathBuf {
    parent.map_or_else(|| root.to_path_buf(), |p| dir_path(root, p))
}

/// Makes a title safe as a file name on all three systems.
pub fn clean_title(title: &str) -> Res<String> {
    let t: String = title
        .chars()
        .map(|c| {
            if "/\\:*?\"<>|".contains(c) || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .collect();
    let t = t
        .trim()
        .trim_start_matches('.')
        .trim_end_matches(['.', ' '])
        .to_string();
    if t.is_empty() {
        return Err("a page needs a name".into());
    }
    Ok(t)
}

fn skip_dir(name: &str, at_root: bool) -> bool {
    name.starts_with('.') || name == "node_modules" || (at_root && (name == ENV_DIR || name == STATE_DIR))
}

pub fn scan(root: &Path) -> Res<Vec<Node>> {
    scan_dir(root, None)
}

fn scan_dir(root: &Path, parent: Option<&str>) -> Res<Vec<Node>> {
    let dir = folder(root, parent);
    let mut titles: Vec<(String, bool, bool)> = Vec::new(); // title, has_md, has_dir
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io(&dir)(e)),
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        let (title, md) = if is_dir {
            if skip_dir(&name, parent.is_none()) {
                continue;
            }
            (name, false)
        } else if let Some(t) = name.strip_suffix(".md") {
            if t.starts_with('.') {
                continue;
            }
            (t.to_string(), true)
        } else {
            continue;
        };
        match titles.iter_mut().find(|(t, _, _)| *t == title) {
            Some(e) => {
                e.1 |= md;
                e.2 |= !md;
            }
            None => titles.push((title, md, !md)),
        }
    }

    let order = read_order(&dir);
    titles.sort_by(|a, b| {
        let pa = order.iter().position(|o| *o == a.0);
        let pb = order.iter().position(|o| *o == b.0);
        match (pa, pb) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.0.to_lowercase().cmp(&b.0.to_lowercase()),
        }
    });

    titles
        .into_iter()
        .map(|(title, has_md, has_dir)| {
            let id = join_id(parent, &title);
            let children = if has_dir {
                scan_dir(root, Some(&id))?
            } else {
                Vec::new()
            };
            Ok(Node {
                id,
                title,
                has_page: has_md,
                children,
            })
        })
        .collect()
}

fn read_order(dir: &Path) -> Vec<String> {
    fs::read_to_string(dir.join(ORDER_FILE))
        .map(|s| {
            s.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Current child titles of a folder, in display order.
fn child_titles(root: &Path, parent: Option<&str>) -> Res<Vec<String>> {
    Ok(scan_dir(root, parent)?.into_iter().map(|n| n.title).collect())
}

fn write_order(root: &Path, parent: Option<&str>, titles: &[String]) -> Res<()> {
    let dir = folder(root, parent);
    let path = dir.join(ORDER_FILE);
    if titles.is_empty() {
        let _ = fs::remove_file(&path);
        return Ok(());
    }
    fs::write(&path, titles.join("\n") + "\n").map_err(io(&path))
}

fn unique_title(root: &Path, parent: Option<&str>, title: &str) -> String {
    let taken = |t: &str| {
        let id = join_id(parent, t);
        md_path(root, &id).exists() || dir_path(root, &id).exists()
    };
    if !taken(title) {
        return title.to_string();
    }
    (2..).map(|n| format!("{title} ({n})")).find(|t| !taken(t)).unwrap()
}

/// Writes atomically: a temp file next to the target, then rename.
pub fn write_atomic(path: &Path, text: &str) -> Res<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io(dir))?;
    }
    let tmp = path.with_extension("md.reqowl-tmp");
    fs::write(&tmp, text).map_err(io(&tmp))?;
    fs::rename(&tmp, path).map_err(io(path))
}

pub fn create_page(root: &Path, parent: Option<&str>, title: &str) -> Res<String> {
    if let Some(p) = parent {
        check_id(p)?;
    }
    let title = unique_title(root, parent, &clean_title(title)?);
    let id = join_id(parent, &title);
    write_atomic(&md_path(root, &id), &format!("# {title}\n\n"))?;
    let mut order = read_order(&folder(root, parent));
    if !order.is_empty() {
        order.push(title);
        write_order(root, parent, &order)?;
    }
    Ok(id)
}

/// Renames a page and its child folder. Returns the new id.
pub fn rename_page(root: &Path, id: &str, new_title: &str) -> Res<String> {
    check_id(id)?;
    let parent = parent_id(id);
    let old_title = title_of(id);
    let clean = clean_title(new_title)?;
    if clean == old_title {
        return Ok(id.to_string());
    }
    let new_title = if clean.eq_ignore_ascii_case(old_title) {
        clean
    } else {
        unique_title(root, parent, &clean)
    };
    let new_id = join_id(parent, &new_title);
    move_files(root, id, &new_id)?;
    let mut order = read_order(&folder(root, parent));
    if let Some(slot) = order.iter_mut().find(|t| *t == old_title) {
        *slot = new_title;
        write_order(root, parent, &order)?;
    }
    Ok(new_id)
}

fn move_files(root: &Path, from: &str, to: &str) -> Res<()> {
    let (fm, tm) = (md_path(root, from), md_path(root, to));
    let (fd, td) = (dir_path(root, from), dir_path(root, to));
    if let Some(p) = tm.parent() {
        fs::create_dir_all(p).map_err(io(p))?;
    }
    if fm.exists() {
        fs::rename(&fm, &tm).map_err(io(&fm))?;
    }
    if fd.is_dir() {
        fs::rename(&fd, &td).map_err(io(&fd))?;
    }
    Ok(())
}

/// Moves a page (with its children) under `new_parent`, placed before the
/// sibling titled `before`, or last. Returns the new id.
pub fn move_page(root: &Path, id: &str, new_parent: Option<&str>, before: Option<&str>) -> Res<String> {
    check_id(id)?;
    if let Some(p) = new_parent {
        check_id(p)?;
        if p == id || p.starts_with(&format!("{id}/")) {
            return Err("a page can't be moved into itself".into());
        }
    }
    let old_parent = parent_id(id);
    let title = title_of(id).to_string();
    let new_id = if old_parent == new_parent {
        id.to_string()
    } else {
        let t = unique_title(root, new_parent, &title);
        let new_id = join_id(new_parent, &t);
        move_files(root, id, &new_id)?;
        // Drop it from the old folder's order.
        let mut old = read_order(&folder(root, old_parent));
        if old.contains(&title) {
            old.retain(|o| *o != title);
            write_order(root, old_parent, &old)?;
        }
        new_id
    };
    let moved_title = title_of(&new_id).to_string();
    let mut order: Vec<String> = child_titles(root, new_parent)?
        .into_iter()
        .filter(|t| *t != moved_title)
        .collect();
    let pos = before
        .and_then(|b| order.iter().position(|t| t == b))
        .unwrap_or(order.len());
    order.insert(pos, moved_title);
    write_order(root, new_parent, &order)?;
    Ok(new_id)
}

/// Moves a page and its children to `.reqowl/trash/<time>/` instead of deleting.
pub fn trash_page(root: &Path, id: &str) -> Res<PathBuf> {
    check_id(id)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let trash = root.join(STATE_DIR).join("trash").join(stamp.to_string());
    fs::create_dir_all(&trash).map_err(io(&trash))?;
    let title = title_of(id);
    let (m, d) = (md_path(root, id), dir_path(root, id));
    if m.exists() {
        fs::rename(&m, trash.join(format!("{title}.md"))).map_err(io(&m))?;
    }
    if d.is_dir() {
        fs::rename(&d, trash.join(title)).map_err(io(&d))?;
    }
    let parent = parent_id(id);
    let mut order = read_order(&folder(root, parent));
    if order.iter().any(|o| o == title) {
        order.retain(|o| o != title);
        write_order(root, parent, &order)?;
    }
    Ok(trash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(nodes: &[Node]) -> Vec<String> {
        nodes
            .iter()
            .flat_map(|n| std::iter::once(n.id.clone()).chain(ids(&n.children)))
            .collect()
    }

    #[test]
    fn tree_ops() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("env")).unwrap();
        fs::create_dir_all(root.join(".reqowl")).unwrap();
        let a = create_page(root, None, "Auth").unwrap();
        let login = create_page(root, Some(&a), "Login").unwrap();
        create_page(root, Some(&a), "Refresh").unwrap();
        create_page(root, None, "Orders").unwrap();
        assert_eq!(create_page(root, None, "Orders").unwrap(), "Orders (2)");
        assert_eq!(create_page(root, None, "a/b:c").unwrap(), "a-b-c");
        assert!(create_page(root, None, " .. ").is_err());
        assert!(check_id("../x").is_err() && check_id("/x").is_err() && check_id("a/./b").is_err());

        assert_eq!(
            ids(&scan(root).unwrap()),
            ["a-b-c", "Auth", "Auth/Login", "Auth/Refresh", "Orders", "Orders (2)"]
        );

        // Reorder within a folder.
        move_page(root, "Auth/Refresh", Some("Auth"), Some("Login")).unwrap();
        assert_eq!(ids(&scan(root).unwrap())[2..4], ["Auth/Refresh", "Auth/Login"]);

        // Move into another page, then rename the parent.
        let moved = move_page(root, &login, Some("Orders"), None).unwrap();
        assert_eq!(moved, "Orders/Login");
        assert!(move_page(root, "Orders", Some("Orders/Login"), None).is_err());
        let renamed = rename_page(root, "Orders", "Shop").unwrap();
        assert_eq!(renamed, "Shop");
        assert!(root.join("Shop/Login.md").exists());

        trash_page(root, "Shop").unwrap();
        assert_eq!(
            ids(&scan(root).unwrap()),
            ["a-b-c", "Auth", "Auth/Refresh", "Orders (2)"]
        );
    }
}
