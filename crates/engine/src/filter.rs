//! Selective sync, exclude globs and the names that never sync.
//!
//! Mirrors `allowed_dir` / `allowed_file` / `excluded` / `SKIP_NAME` from the
//! Python engine. Python's `fnmatch` lets `*` cross `/`; `glob::Pattern` is
//! configured that way by default, so the two agree on the usual globs.

use glob::Pattern;
use regex::Regex;
use std::sync::OnceLock;

fn skip_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"^\.Trash-\d+$|^\.~lock\..*#$|^~\$.*|^\.#.*|^\..*\.sw[a-p]$|^\._.*|^\.DS_Store$",
        )
        .unwrap()
    })
}

#[derive(Clone, Debug, Default)]
pub struct Filter {
    subfolders: Vec<String>,
    exclude: Vec<Pattern>,
}

impl Filter {
    pub fn new(subfolders: &str, exclude: &str) -> Self {
        let subfolders = subfolders
            .split(',')
            .map(|s| s.trim().trim_matches('/').to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let exclude = exclude
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|p| Pattern::new(p).unwrap_or_else(|_| Pattern::new(&Pattern::escape(p)).unwrap()))
            .collect();
        Self { subfolders, exclude }
    }

    /// Names that must never sync anywhere (editor/OS lock-and-swap junk).
    pub fn skip_name(name: &str) -> bool {
        skip_re().is_match(name)
    }

    /// This folder is in view if selected, contains a selection, or is an
    /// ancestor of one. The root is always in view.
    pub fn allowed_dir(&self, rel: &str) -> bool {
        if self.subfolders.is_empty() || rel.is_empty() {
            return true;
        }
        self.subfolders
            .iter()
            .any(|s| rel == s || rel.starts_with(&format!("{s}/")) || s.starts_with(&format!("{rel}/")))
    }

    pub fn allowed_file(&self, rel: &str) -> bool {
        if self.subfolders.is_empty() {
            return true;
        }
        self.subfolders
            .iter()
            .any(|s| rel == s || rel.starts_with(&format!("{s}/")))
    }

    /// True when nothing limits the walk (no selective sync, no excludes), so
    /// a snapshot of it is a complete view of the remote tree.
    pub fn is_unfiltered(&self) -> bool {
        self.subfolders.is_empty() && self.exclude.is_empty()
    }

    /// Signature of the walk settings (mirrors `walk_signature` in `core`).
    pub fn signature(&self, remote_root: &str) -> String {
        let subs = self.subfolders.join(",");
        let excl = self
            .exclude
            .iter()
            .map(|p| p.as_str())
            .collect::<Vec<_>>()
            .join(",");
        format!("{remote_root}|{subs}|{excl}")
    }

    pub fn excluded(&self, rel: &str) -> bool {
        if self.exclude.is_empty() {
            return false;
        }
        let base = rel.rsplit('/').next().unwrap_or(rel);
        self.exclude
            .iter()
            .any(|p| p.matches(rel) || p.matches(base))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_lock_and_swap_artifacts_everywhere() {
        for n in [
            ".Trash-1000",
            ".~lock.Report.docx#",
            "~$Report.docx",
            ".#emacs",
            ".file.swp",
            "._resource",
            ".DS_Store",
        ] {
            assert!(Filter::skip_name(n), "{n}");
        }
        for n in ["Report.docx", "report.swp", ".gitignore", "notes.txt"] {
            assert!(!Filter::skip_name(n), "{n}");
        }
    }

    #[test]
    fn selective_sync_keeps_ancestors_and_selected_subtrees() {
        let f = Filter::new("Docs,Photos/2026", "");
        assert!(f.allowed_dir(""));
        assert!(f.allowed_dir("Docs"));
        assert!(f.allowed_dir("Docs/Sub"));
        assert!(f.allowed_dir("Photos"));
        assert!(f.allowed_dir("Photos/2026"));
        assert!(!f.allowed_dir("Videos"));
        assert!(f.allowed_file("Docs/a.txt"));
        assert!(f.allowed_file("Docs"));
        assert!(f.allowed_file("Photos/2026/b.jpg"));
        assert!(!f.allowed_file("Photos/2025/b.jpg"));
        assert!(!f.allowed_file("Videos/a.mp4"));
    }

    #[test]
    fn unfiltered_walks_are_the_only_complete_snapshots() {
        assert!(Filter::new("", "").is_unfiltered());
        assert!(!Filter::new("Docs", "").is_unfiltered());
        assert!(!Filter::new("", "*.tmp").is_unfiltered());
        assert_eq!(
            Filter::new("Docs,Photos", "").signature("/my-files"),
            "/my-files|Docs,Photos|"
        );
    }

    #[test]
    fn globs_match_path_and_basename() {
        let f = Filter::new("", "*.tmp,.cache,secret/*.txt");
        assert!(f.excluded("a/b.tmp"));
        assert!(f.excluded("x/.cache"));
        assert!(f.excluded("secret/x.txt"));
        assert!(!f.excluded("a/b.txt"));
    }

    #[test]
    fn empty_filter_allows_everything() {
        let f = Filter::new("", "");
        assert!(f.allowed_dir("anything"));
        assert!(f.allowed_file("anything"));
        assert!(!f.excluded("anything"));
    }
}
