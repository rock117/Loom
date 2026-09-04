//! Include / exclude glob filters for recursive SFTP transfers.

use globset::{Glob, GlobSet, GlobSetBuilder};

/// Default directory/file names skipped unless the user unchecks them.
pub const DEFAULT_EXCLUDE_PRESETS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    ".git",
    "__pycache__",
    ".next",
    "out",
    ".turbo",
    ".cache",
];

/// Transfer path filters. Empty `include` means “everything not excluded”.
#[derive(Debug, Clone, Default)]
pub struct TransferFilter {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

impl TransferFilter {
    pub fn from_lists(include: Vec<String>, exclude: Vec<String>) -> Self {
        Self {
            include: normalize_patterns(include),
            exclude: normalize_patterns(exclude),
        }
    }

    /// Default filter: no include restriction, build-artifact presets excluded.
    pub fn with_default_excludes() -> Self {
        Self {
            include: Vec::new(),
            exclude: DEFAULT_EXCLUDE_PRESETS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.include.is_empty() && self.exclude.is_empty()
    }

    pub fn matcher(&self) -> anyhow::Result<FilterMatcher> {
        FilterMatcher::new(self)
    }
}

fn normalize_patterns(patterns: Vec<String>) -> Vec<String> {
    patterns
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// Compiled glob matcher. Paths are matched as relative POSIX-style strings
/// (`foo/bar.rs`) plus basename-only checks for simple patterns like `target`.
pub struct FilterMatcher {
    include: Option<GlobSet>,
    exclude: GlobSet,
    /// Raw include patterns (for basename fallback).
    include_raw: Vec<String>,
    exclude_raw: Vec<String>,
}

impl FilterMatcher {
    pub fn new(filter: &TransferFilter) -> anyhow::Result<Self> {
        let include = if filter.include.is_empty() {
            None
        } else {
            Some(build_set(&filter.include)?)
        };
        let exclude = build_set(&filter.exclude)?;
        Ok(Self {
            include,
            exclude,
            include_raw: filter.include.clone(),
            exclude_raw: filter.exclude.clone(),
        })
    }

    /// Whether this relative path (file or dir) should be transferred.
    /// `rel` uses `/` separators; empty string = transfer root itself (always kept).
    pub fn allows(&self, rel: &str, is_dir: bool) -> bool {
        if rel.is_empty() {
            return true;
        }
        let rel = rel.trim_start_matches('/');
        let base = basename(rel);

        if self.matches_exclude(rel, base) {
            return false;
        }

        match &self.include {
            None => true,
            Some(set) => {
                if set.is_match(rel) || set.is_match(base) {
                    return true;
                }
                // Directories: keep if any include pattern could live underneath.
                if is_dir {
                    return self.include_raw.iter().any(|pat| dir_may_contain(rel, pat));
                }
                false
            }
        }
    }

    fn matches_exclude(&self, rel: &str, base: &str) -> bool {
        if self.exclude.is_match(rel) || self.exclude.is_match(base) {
            return true;
        }
        // Path segment equal to a simple exclude name (e.g. `…/node_modules/…`).
        for pat in &self.exclude_raw {
            if is_simple_name(pat) {
                for seg in rel.split('/') {
                    if seg == pat.as_str() {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub fn include_patterns(&self) -> Vec<String> {
        self.include_raw.clone()
    }

    pub fn exclude_patterns(&self) -> Vec<String> {
        self.exclude_raw.clone()
    }
}

fn build_set(patterns: &[String]) -> anyhow::Result<GlobSet> {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        // Allow both `*.rs` and `**/*.rs`-style; also accept Windows `\`.
        let norm = p.replace('\\', "/");
        let glob = Glob::new(&norm)
            .or_else(|_| Glob::new(&format!("**/{norm}")))
            .map_err(|e| anyhow::anyhow!("bad glob `{p}`: {e}"))?;
        b.add(glob);
    }
    b.build()
        .map_err(|e| anyhow::anyhow!("globset build: {e}"))
}

fn basename(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

fn is_simple_name(pat: &str) -> bool {
    !pat.contains('/') && !pat.contains('\\') && !pat.contains('*') && !pat.contains('?') && !pat.contains('[')
}

/// Heuristic: keep walking a directory if an include pattern could match under it.
fn dir_may_contain(dir: &str, pat: &str) -> bool {
    let pat = pat.replace('\\', "/");
    if pat.starts_with(dir) && (pat.len() == dir.len() || pat.as_bytes().get(dir.len()) == Some(&b'/')) {
        return true;
    }
    // Pattern like `**/foo` or `*.rs` — keep dirs.
    if pat.contains('*') || pat.contains('?') {
        return true;
    }
    false
}

/// Parse a comma / newline / semicolon separated pattern list.
pub fn parse_pattern_list(text: &str) -> Vec<String> {
    text.split(|c| c == ',' || c == ';' || c == '\n' || c == '\r')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_excludes_node_modules() {
        let m = TransferFilter::with_default_excludes().matcher().unwrap();
        assert!(!m.allows("pkg/node_modules", true));
        assert!(!m.allows("pkg/node_modules/x.js", false));
        assert!(m.allows("pkg/src/main.rs", false));
    }

    #[test]
    fn include_glob() {
        let f = TransferFilter::from_lists(vec!["**/*.rs".into()], vec![]);
        let m = f.matcher().unwrap();
        assert!(m.allows("src/main.rs", false));
        assert!(!m.allows("src/main.js", false));
        assert!(m.allows("src", true)); // may contain
    }

    #[test]
    fn exclude_wildcard() {
        let f = TransferFilter::from_lists(vec![], vec!["*.o".into(), "target".into()]);
        let m = f.matcher().unwrap();
        assert!(!m.allows("a.o", false));
        assert!(!m.allows("lib/target/x", false));
        assert!(m.allows("a.rs", false));
    }
}
