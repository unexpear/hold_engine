//! Path canonicalisation + extension allowlist for the hot-reload watcher.
//!
//! Two concerns, both pure logic and fully testable without spinning
//! up a Win32 directory watch:
//!
//! 1. **Canonicalisation** turns an arbitrary OS path into a stable
//!    form the rest of the pipeline can compare against. On Windows
//!    that means: absolute, normalised separators (`/` → `\`), and
//!    lowercased — the file-system is case-insensitive but
//!    `ReadDirectoryChangesW` reports paths with whatever case
//!    happened to be cached, so without lowercasing the reverse-index
//!    lookup misses on `Foo.PNG` when the project knows it as
//!    `foo.png`. See ADR 0004 Q4.
//!
//! 2. **Extension allowlist** filters out file events the engine
//!    doesn't care about. Project source trees include `.tmp`, IDE
//!    swap files, `.git/` traffic, etc. — letting all of those flow
//!    into the compile queue produces noise and burns the coalescing
//!    budget. The allowlist is the watcher's first line of defence.

use std::path::{Path, PathBuf};

/// Canonicalise a Windows file-system path to the form the hot-reload
/// pipeline compares against.
///
/// Behaviour:
/// - The path is left absolute as given. The watcher passes absolute
///   paths in (joined with the watch root); higher layers don't have
///   to think about `current_dir`.
/// - Forward slashes become backslashes (`a/b/c.png` → `a\b\c.png`).
/// - Letters are lowercased: Windows is case-insensitive at the
///   file-system layer but `ReadDirectoryChangesW` honours whatever
///   case the OS happened to cache; the reverse-index lookup must
///   not depend on that cache.
/// - Trailing whitespace and dots are stripped; Windows treats
///   `foo.txt.` and `foo.txt ` as the same file as `foo.txt`, so
///   different reported variants must canonicalise to the same key.
///
/// Pure function — no syscalls, no allocation beyond the returned
/// `PathBuf`.
pub fn canonicalise_for_index(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    // 1. Normalise separators.
    // 2. Lowercase.
    // 3. Strip trailing whitespace + dots.
    let mut out: String = s
        .chars()
        .map(|c| match c {
            '/' => '\\',
            other => other.to_ascii_lowercase(),
        })
        .collect();
    while matches!(out.chars().last(), Some(' ') | Some('.')) {
        out.pop();
    }
    PathBuf::from(out)
}

/// A configurable allowlist of file extensions the watcher cares
/// about. Pass it lowercase, no leading dot (`png`, `glsl`, `obj`).
///
/// Lookup is case-insensitive: a path ending in `.PNG`, `.Png`, or
/// `.png` all match an allowlist entry of `png`.
#[derive(Debug, Clone)]
pub struct ExtensionFilter {
    extensions: Vec<String>,
}

impl ExtensionFilter {
    /// Build a filter from an iterator of extension strings. Each
    /// string is normalised: leading dots are stripped (so `.png`
    /// and `png` both work), and the entry is lowercased. Duplicates
    /// are deduplicated.
    pub fn new<I, S>(extensions: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut normalised: Vec<String> = extensions
            .into_iter()
            .map(|raw| {
                let s: String = raw.into();
                let trimmed = s.trim_start_matches('.');
                trimmed.to_ascii_lowercase()
            })
            .filter(|s| !s.is_empty())
            .collect();
        normalised.sort();
        normalised.dedup();
        Self {
            extensions: normalised,
        }
    }

    /// The default asset-extension set the engine ships with. Covers
    /// every file kind the M5 asset pipeline understands, plus the
    /// scene format (`hsc`) and the proposed material source files
    /// from ADR 0002. Adding a new asset kind to the engine should
    /// extend this list, not silently rely on case-by-case allowlists
    /// scattered around the codebase.
    pub fn default_engine_extensions() -> Self {
        Self::new([
            "bmp", "tga", "png", // textures (BMP/TGA shipped, PNG planned)
            "obj",  // meshes
            "glsl", "vert", "frag", // raw GLSL fragments (legacy)
            "hshd", // hot-reloadable shader source pair (M7.4.1)
            "hsc",  // scenes
            "material", "hmat", // material graph definitions (M6;
                              // .hmat is the canonical extension
                              // matching the asset compiler's
                              // resolve_source_asset_path logic)
            "hcmp", // compiled-asset envelope (rare; mostly for
                    // recompile-after-format-change cases)
        ])
    }

    /// Whether `path` ends with a recognised extension. The lookup
    /// runs in O(log N) over the allowlist size. For typical N (10–20
    /// extensions) the constant factor is dominated by string
    /// comparison, so this is fine even on hot paths.
    pub fn accepts(&self, path: &Path) -> bool {
        let ext = match path.extension() {
            Some(e) => e,
            None => return false,
        };
        let lower = ext.to_string_lossy().to_ascii_lowercase();
        self.extensions.binary_search(&lower).is_ok()
    }

    /// Read-only view of the configured extension list.
    pub fn extensions(&self) -> &[String] {
        &self.extensions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── canonicalise_for_index ─────────────────────────────────────────────

    #[test]
    fn canonicalise_lowercases_path() {
        let input = Path::new("C:\\Project\\Source\\Foo.PNG");
        let output = canonicalise_for_index(input);
        assert_eq!(output.to_string_lossy(), "c:\\project\\source\\foo.png");
    }

    #[test]
    fn canonicalise_normalises_separators() {
        let input = Path::new("C:/Project/Source/foo.png");
        let output = canonicalise_for_index(input);
        assert_eq!(output.to_string_lossy(), "c:\\project\\source\\foo.png");
    }

    /// Windows treats `foo.txt.` and `foo.txt ` as the same file as
    /// `foo.txt`. The canonicaliser must collapse them so the
    /// reverse-index lookup hits the same key regardless of which
    /// variant the OS reports.
    #[test]
    fn canonicalise_strips_trailing_dots_and_spaces() {
        for raw in [
            "C:\\Foo\\bar.txt",
            "C:\\Foo\\bar.txt.",
            "C:\\Foo\\bar.txt..",
            "C:\\Foo\\bar.txt ",
            "C:\\Foo\\bar.txt . . ",
        ] {
            let canon = canonicalise_for_index(Path::new(raw));
            assert_eq!(
                canon.to_string_lossy(),
                "c:\\foo\\bar.txt",
                "input {:?} canonicalised wrong",
                raw
            );
        }
    }

    /// Different cased reports of the same path must produce the
    /// same canonical form. This is the property the reverse-index
    /// lookup depends on.
    #[test]
    fn canonicalise_is_idempotent_under_case_variants() {
        let variants = [
            "C:\\Project\\Source\\foo.png",
            "c:\\PROJECT\\source\\Foo.PNG",
            "C:/Project/Source/FOO.png",
            "c:/project/source/foo.PNG",
        ];
        let canonical = canonicalise_for_index(Path::new(variants[0]));
        for v in &variants[1..] {
            assert_eq!(canonicalise_for_index(Path::new(v)), canonical);
        }
    }

    /// Canonicalising an already-canonical path returns the same
    /// path. Important so repeated canonicalisation in the pipeline
    /// is a no-op rather than a slow drift.
    #[test]
    fn canonicalise_is_fixed_point_on_canonical_input() {
        let canon = PathBuf::from("c:\\foo\\bar.txt");
        let twice = canonicalise_for_index(&canon);
        assert_eq!(canon, twice);
    }

    // ── ExtensionFilter ────────────────────────────────────────────────────

    #[test]
    fn extension_filter_accepts_known_kinds() {
        let filter = ExtensionFilter::new(["png", "obj", "glsl"]);
        assert!(filter.accepts(Path::new("a/b/foo.png")));
        assert!(filter.accepts(Path::new("foo.obj")));
        assert!(filter.accepts(Path::new("shaders/lit.glsl")));
    }

    #[test]
    fn extension_filter_is_case_insensitive() {
        let filter = ExtensionFilter::new(["png"]);
        assert!(filter.accepts(Path::new("foo.PNG")));
        assert!(filter.accepts(Path::new("foo.Png")));
        assert!(filter.accepts(Path::new("foo.pNg")));
    }

    #[test]
    fn extension_filter_rejects_unknown_extensions() {
        let filter = ExtensionFilter::new(["png"]);
        assert!(!filter.accepts(Path::new("foo.tmp")));
        assert!(!filter.accepts(Path::new("foo.swp")));
        assert!(!filter.accepts(Path::new("foo")));
        assert!(!filter.accepts(Path::new("foo.png.bak")));
    }

    /// Filter normalises input: leading dots stripped, lowercased,
    /// duplicates deduplicated. The user can pass `.png` or `png` or
    /// even `.PNG` and get the same filter behaviour.
    #[test]
    fn extension_filter_normalises_input_extensions() {
        let filter = ExtensionFilter::new([".png", "PNG", "Png", ".OBJ"]);
        // Two distinct entries after normalisation: png, obj.
        assert_eq!(filter.extensions().len(), 2);
        assert!(filter.accepts(Path::new("foo.png")));
        assert!(filter.accepts(Path::new("foo.obj")));
    }

    /// Empty filter rejects everything — useful for tests that want
    /// to verify "all events filtered out" paths.
    #[test]
    fn empty_extension_filter_rejects_everything() {
        let filter = ExtensionFilter::new(std::iter::empty::<&str>());
        assert!(!filter.accepts(Path::new("foo.png")));
        assert!(!filter.accepts(Path::new("foo")));
    }

    /// Default engine extensions cover every asset kind M5 ships
    /// (textures, meshes, shaders, scenes, materials). Locks the
    /// "we didn't accidentally drop a kind" property — adding a
    /// new asset kind to the engine should extend this list.
    #[test]
    fn default_engine_extensions_cover_shipped_asset_kinds() {
        let filter = ExtensionFilter::default_engine_extensions();
        for ext in ["bmp", "tga", "obj", "glsl", "hsc", "material"] {
            assert!(
                filter.accepts(Path::new(&format!("foo.{}", ext))),
                "default filter missing extension {}",
                ext
            );
        }
    }

    /// `.hshd` (hot-reloadable shader source pair) is in the
    /// default filter — without it, M7.4.1 shader hot-reload
    /// silently never triggers because the file watcher's first
    /// line of defence drops every save event before it reaches
    /// the compile queue.
    #[test]
    fn default_extensions_include_hshd_for_shader_hot_reload() {
        let filter = ExtensionFilter::default_engine_extensions();
        assert!(filter.accepts(Path::new("shaders/textured-pbr.hshd")));
    }
}
