#![recursion_limit = "256"]
//! D2H core library — directory scanning and HTML generation.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const APP_NAME: &str = "D2H";
pub const APP_VERSION: &str = "1.6.2-beta";
pub const CHUNK_SIZE: usize = 100;

const PACKAGE_EXTENSIONS: &[&str] = &[
    ".app",
    ".photoslibrary",
    ".photolibrary",
    ".aplibrary",
    ".imovielibrary",
    ".pages",
    ".numbers",
    ".keynote",
    ".rtfd",
];

// Templates embedded at compile time
pub const CSS_TEMPLATE: &str = include_str!("../templates/style.css");
pub const JS_TEMPLATE: &str = include_str!("../templates/app.js");
pub const HTML_TEMPLATE: &str = include_str!("../templates/single.html");
pub const MULTI_HTML_TEMPLATE: &str = include_str!("../templates/multi.html");
pub const EXTERNAL_LOADER_JS: &str = include_str!("../templates/external_loader.js");
pub const PAKO_JS: &str = include_str!("../templates/pako.min.js");
pub const LOGO_SVG: &[u8] = include_bytes!("../templates/logo.svg");

// ============================================================================
// PUBLIC TYPES
// ============================================================================

/// Options for directory scanning (framework-agnostic, no CLI dependency)
#[derive(Debug, Clone)]
pub struct ScanOptions {
    pub dir: PathBuf,
    pub hidden: bool,
    pub include_zero_size: bool,
    pub include_empty_dirs: bool,
    pub follow_symlinks: bool,
    pub show_packages: bool,
}

/// Options for output generation.
/// Logo and pako are optional overrides — if None, the embedded defaults are used.
#[derive(Debug, Clone)]
pub struct OutputOptions {
    pub logo: Option<PathBuf>,
    pub pako: Option<PathBuf>,
    pub lang: String,  // "en", "cz", "de", "it", "es", "fr", "pl"
    pub theme: String, // "default", "nologo"
    /// Custom page background (any CSS background value) — overrides theme CSS.
    pub header_background: Option<String>,
    /// Pre-rendered footer HTML ({{FOOTER_HTML}}); build with build_footer_html().
    pub footer_html: String,
}

impl Default for OutputOptions {
    fn default() -> Self {
        Self {
            logo: None,
            pako: None,
            lang: "en".to_string(),
            theme: "default".to_string(),
            header_background: None,
            footer_html: build_footer_html(&Config::default()),
        }
    }
}

// ============================================================================
// CONFIGURATION (d2h.config.json)
// ============================================================================
// All site-specific behavior (branding, default paths, job-number detection
// rules) lives in an optional JSON config file — the binary itself is neutral.
// Search order: explicit path → d2h.config.json next to the executable →
// user config dir (%APPDATA%\d2h\config.json or ~/.config/d2h/config.json).

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub branding: Branding,
    pub defaults: Defaults,
    pub job_id: JobIdConfig,
    /// Directory the config file was loaded from — used to resolve relative
    /// logo paths. Not part of the JSON.
    #[serde(skip)]
    pub base_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Branding {
    /// Path to a custom logo file (PNG or SVG) used in the generated output.
    pub logo_path: Option<String>,
    /// Page background as any CSS value (color, gradient…); wins over theme.
    pub header_background: Option<String>,
    /// Footer text shown in the generated output.
    pub footer_text: String,
    /// Footer link target; empty string = plain text footer.
    pub footer_url: String,
    pub show_footer: bool,
}

impl Default for Branding {
    fn default() -> Self {
        Self {
            logo_path: None,
            header_background: None,
            footer_text: "Generated with D2H".to_string(),
            footer_url: "https://www.datahelp.eu".to_string(),
            show_footer: true,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Defaults {
    pub language: String,
    pub theme: String,
    /// UI language override for the GUI (en/cz/de/it/es/fr/pl).
    /// None = auto-detect from the OS locale.
    pub ui_language: Option<String>,
    pub web_output_dir: Option<String>,
    pub html_output_dir: Option<String>,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            language: "en".to_string(),
            theme: "default".to_string(),
            ui_language: None,
            web_output_dir: None,
            html_output_dir: None,
        }
    }
}

/// Job/case-number detection. Without a `pattern` nothing is detected and no
/// validation dialog ever appears — that is the neutral public default.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct JobIdConfig {
    /// Regex matching a valid job number (e.g. "JOB-\\d{4}").
    pub pattern: Option<String>,
    /// Regex for names that LOOK like a job number — a full match that fails
    /// `pattern` triggers the "invalid job number" dialog (e.g. "\\d+").
    pub warn_pattern: Option<String>,
    /// Human-readable description of the expected format, shown in dialogs.
    pub description: Option<String>,
    /// Auto-default rules applied by the GUI when a source dir is picked.
    /// First matching rule wins.
    pub rules: Vec<JobIdRule>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct JobIdRule {
    /// Regex tested against the source directory name.
    pub r#match: String,
    pub language: Option<String>,
    pub theme: Option<String>,
    pub web_output_dir: Option<String>,
    pub html_output_dir: Option<String>,
    /// Per-rule branding overrides (e.g. a different logo/background for
    /// one class of jobs). logo_path may be relative to the config file.
    pub logo_path: Option<String>,
    pub header_background: Option<String>,
}

impl Config {
    /// Load config. Returns (config, path-it-was-loaded-from).
    /// An explicit path that doesn't exist is an error; otherwise a missing
    /// config silently yields defaults.
    pub fn load(explicit: Option<&Path>) -> Result<(Self, Option<PathBuf>), String> {
        if let Some(p) = explicit {
            if !p.exists() {
                return Err(format!("Config file '{}' not found.", p.display()));
            }
            return Self::read_file(p).map(|c| (c, Some(p.to_path_buf())));
        }

        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                candidates.push(dir.join("d2h.config.json"));
            }
        }
        if let Some(base) = config_base_dir() {
            candidates.push(base.join("d2h").join("config.json"));
        }
        for c in candidates {
            if c.exists() {
                return Self::read_file(&c).map(|cfg| (cfg, Some(c)));
            }
        }
        Ok((Config::default(), None))
    }

    fn read_file(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path)
            .map_err(|e| format!("Cannot read config '{}': {}", path.display(), e))?;
        let mut cfg: Config = serde_json::from_str(&text)
            .map_err(|e| format!("Invalid config '{}': {}", path.display(), e))?;
        cfg.base_dir = path.parent().map(|p| p.to_path_buf());
        Ok(cfg)
    }
}

fn config_base_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA").ok().map(PathBuf::from)
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("XDG_CONFIG_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".config")))
    }
}

/// Check if a name is a valid job number per the configured pattern
/// (anchored full match). No pattern configured → always false.
pub fn is_valid_job_id(name: &str, cfg: &Config) -> bool {
    match cfg.job_id.pattern.as_deref() {
        Some(p) => anchored_match(p, name),
        None => false,
    }
}

/// True when the name LOOKS like a job number (warn_pattern full match)
/// but is not a valid one — the GUI shows a validation dialog then.
pub fn looks_like_invalid_job_id(name: &str, cfg: &Config) -> bool {
    match cfg.job_id.warn_pattern.as_deref() {
        Some(w) => anchored_match(w, name) && !is_valid_job_id(name, cfg),
        None => false,
    }
}

fn anchored_match(pattern: &str, s: &str) -> bool {
    regex::Regex::new(&format!("^(?:{})$", pattern))
        .map(|re| re.is_match(s))
        .unwrap_or(false)
}

/// Extract a job number from a directory name using the configured pattern.
/// The match must start at position 0 and must not be immediately followed by
/// another digit (so "573456" does not yield "57345").
/// Returns `Some((job_id, is_exact))`; `is_exact` = the whole name matched.
pub fn extract_job_id(dir_name: &str, cfg: &Config) -> Option<(String, bool)> {
    let pattern = cfg.job_id.pattern.as_deref()?;
    let re = regex::Regex::new(pattern).ok()?;
    let m = re.find(dir_name)?;
    if m.start() != 0 {
        return None;
    }
    let exact = m.end() == dir_name.len();
    let followed_by_digit = dir_name[m.end()..]
        .chars()
        .next()
        .map_or(false, |c| c.is_ascii_digit());
    if !exact && followed_by_digit {
        return None;
    }
    Some((m.as_str().to_string(), exact))
}

/// Resolve a possibly-relative path from the config against the config
/// file's directory.
pub fn resolve_config_path(cfg: &Config, p: &str) -> PathBuf {
    let path = PathBuf::from(p);
    if path.is_absolute() {
        return path;
    }
    match &cfg.base_dir {
        Some(base) => base.join(path),
        None => path,
    }
}

/// First jobId rule whose regex matches the directory name (same semantics
/// as the GUI auto-defaults — first match wins).
pub fn find_matching_rule<'a>(dir_name: &str, cfg: &'a Config) -> Option<&'a JobIdRule> {
    cfg.job_id.rules.iter().find(|rule| {
        regex::Regex::new(&rule.r#match)
            .map(|re| re.is_match(dir_name))
            .unwrap_or(false)
    })
}

/// Render the attribution footer for the generated output.
pub fn build_footer_html(cfg: &Config) -> String {
    if !cfg.branding.show_footer {
        return String::new();
    }
    let text = html_escape(&cfg.branding.footer_text);
    if cfg.branding.footer_url.is_empty() {
        format!("<div class=\"app_footer\">{}</div>", text)
    } else {
        format!(
            "<div class=\"app_footer\"><a href=\"{}\" target=\"_blank\" rel=\"noopener\">{}</a></div>",
            html_escape(&cfg.branding.footer_url),
            text
        )
    }
}

/// Build OutputOptions pre-filled from a Config (logo, background, footer).
pub fn output_options_from_config(cfg: &Config, lang: &str, theme: &str) -> OutputOptions {
    OutputOptions {
        logo: cfg.branding.logo_path.as_deref().map(|p| resolve_config_path(cfg, p)),
        pako: None,
        lang: lang.to_string(),
        theme: theme.to_string(),
        header_background: cfg.branding.header_background.clone(),
        footer_html: build_footer_html(cfg),
    }
}

#[derive(Debug)]
pub struct DirEntry {
    pub id: usize,
    pub parent_id: Option<usize>,
    pub path: PathBuf,
    pub rel_path: String,
    pub modified: i64,
    pub subdirs: Vec<usize>,
    pub content: Vec<ContentItem>,
}

#[derive(Debug)]
pub enum ContentItem {
    Folder {
        title: String,
        key: usize,
        is_lazy: bool,
        modified: i64,
        size: u64,
    },
    File {
        title: String,
        size: u64,
        modified: i64,
    },
}

/// Result of directory scan
pub struct ScanResult {
    pub dirs: BTreeMap<usize, DirEntry>,
    pub total_files: u64,
    pub total_dirs: u64,
    pub total_size: u64,
    pub warnings: Vec<String>,
    /// Extension stats: ext → (count, total_size)
    pub extension_stats: HashMap<String, (u64, u64)>,
}

/// Per-directory subtree statistics (pre-computed for chunk embedding)
pub struct SubtreeStats {
    pub total_files: u64,
    pub total_dirs: u64,
    pub total_size: u64,
    pub extension_stats: HashMap<String, (u64, u64)>,
}

/// Compute subtree stats for every directory (bottom-up aggregation).
pub fn compute_subtree_stats(dirs: &BTreeMap<usize, DirEntry>) -> BTreeMap<usize, SubtreeStats> {
    let mut stats_map: BTreeMap<usize, SubtreeStats> = BTreeMap::new();

    // Process in reverse ID order (leaves first, root last)
    for (&id, dir) in dirs.iter().rev() {
        let mut st = SubtreeStats {
            total_files: 0,
            total_dirs: 0,
            total_size: 0,
            extension_stats: HashMap::new(),
        };

        // Count direct files
        for item in &dir.content {
            if let ContentItem::File { title, size, .. } = item {
                st.total_files += 1;
                st.total_size += *size;
                let ext = Path::new(title)
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                let entry = st.extension_stats.entry(ext).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += *size;
            }
        }

        // Merge child subtree stats
        for &sub_id in &dir.subdirs {
            if let Some(child_stats) = stats_map.get(&sub_id) {
                st.total_files += child_stats.total_files;
                st.total_dirs += child_stats.total_dirs + 1;
                st.total_size += child_stats.total_size;
                for (ext, (count, size)) in &child_stats.extension_stats {
                    let entry = st.extension_stats.entry(ext.clone()).or_insert((0, 0));
                    entry.0 += *count;
                    entry.1 += *size;
                }
            }
        }

        stats_map.insert(id, st);
    }

    stats_map
}

// ============================================================================
// SCANNING
// ============================================================================

pub fn scan_directory(opts: &ScanOptions, progress: Option<&dyn Fn(&str)>) -> ScanResult {
    let root = &opts.dir;
    let mut dirs: BTreeMap<usize, DirEntry> = BTreeMap::new();
    let mut next_id: usize = 0;
    let mut path_to_id: HashMap<PathBuf, usize> = HashMap::new();
    let mut total_files: u64 = 0;
    let mut total_size: u64 = 0;
    let mut warnings: Vec<String> = Vec::new();
    let mut visited_real_paths: HashSet<PathBuf> = HashSet::new();
    let mut extension_stats: HashMap<String, (u64, u64)> = HashMap::new();

    let root_id = next_id;
    next_id += 1;
    let root_path = long_path(root);
    let root_modified = get_mtime(&root_path);

    if let Ok(real) = fs::canonicalize(&root_path) {
        visited_real_paths.insert(real);
    }

    dirs.insert(
        root_id,
        DirEntry {
            id: root_id,
            parent_id: None,
            path: root.to_path_buf(),
            rel_path: String::new(),
            modified: root_modified,
            subdirs: Vec::new(),
            content: Vec::new(),
        },
    );
    path_to_id.insert(root.to_path_buf(), root_id);

    let mut stack: Vec<(PathBuf, usize)> = vec![(root.to_path_buf(), root_id)];

    while let Some((dir_path, dir_id)) = stack.pop() {
        let scan_path = long_path(&dir_path);
        let entries = match fs::read_dir(&scan_path) {
            Ok(e) => e,
            Err(e) => {
                warnings.push(format!("Cannot read '{}': {}", dir_path.display(), e));
                continue;
            }
        };

        let mut sub_folders: Vec<(String, PathBuf, i64)> = Vec::new();
        let mut files: Vec<(String, u64, i64)> = Vec::new();

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    warnings.push(format!(
                        "Cannot read entry in '{}': {}",
                        dir_path.display(),
                        e
                    ));
                    continue;
                }
            };

            let entry_path = entry.path();
            let clean_path = strip_long_path(&entry_path);
            let name = entry.file_name().to_string_lossy().to_string();

            // Hidden file detection
            if !opts.hidden {
                if name.starts_with('.') {
                    continue;
                }
                #[cfg(target_os = "windows")]
                {
                    if is_hidden_windows(&entry_path, &mut warnings) {
                        continue;
                    }
                }
            }

            // Get metadata
            let metadata = if opts.follow_symlinks {
                match fs::metadata(&entry_path) {
                    Ok(m) => m,
                    Err(e) => {
                        warnings.push(format!(
                            "Cannot stat '{}': {} (broken symlink?)",
                            clean_path.display(),
                            e
                        ));
                        continue;
                    }
                }
            } else {
                match fs::symlink_metadata(&entry_path) {
                    Ok(m) => m,
                    Err(e) => {
                        warnings.push(format!(
                            "Cannot stat '{}': {}",
                            clean_path.display(),
                            e
                        ));
                        continue;
                    }
                }
            };

            // Symlinks in no-follow mode
            let file_type = metadata.file_type();
            if file_type.is_symlink() && !opts.follow_symlinks {
                if let Ok(target_meta) = fs::metadata(&entry_path) {
                    if target_meta.is_dir() {
                        continue;
                    }
                    let size = target_meta.len();
                    if !opts.include_zero_size && size == 0 {
                        continue;
                    }
                    let mtime = get_mtime_from_metadata(&target_meta);
                    let ext = Path::new(&name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                    let entry = extension_stats.entry(ext).or_insert((0, 0));
                    entry.0 += 1;
                    entry.1 += size;
                    files.push((name, size, mtime));
                    total_files += 1;
                    total_size += size;
                } else {
                    if opts.include_zero_size {
                        let ext = Path::new(&name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                        let entry = extension_stats.entry(ext).or_insert((0, 0));
                        entry.0 += 1;
                        files.push((name, 0, 0));
                        total_files += 1;
                    }
                }
                continue;
            }

            if metadata.is_dir() {
                // macOS package detection
                if !opts.show_packages && is_macos_package(&name) {
                    let pkg_size = calculate_dir_size(&entry_path, &mut warnings);
                    let mtime = get_mtime_from_metadata(&metadata);
                    let ext = Path::new(&name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                    let entry = extension_stats.entry(ext).or_insert((0, 0));
                    entry.0 += 1;
                    entry.1 += pkg_size;
                    files.push((name, pkg_size, mtime));
                    total_files += 1;
                    total_size += pkg_size;
                    continue;
                }

                // Symlink cycle detection — only when following symlinks
                // Without symlinks, filesystem cycles are impossible
                if opts.follow_symlinks {
                    // Check if this entry is actually a symlink
                    let is_symlink = fs::symlink_metadata(&entry_path)
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(false);

                    if is_symlink {
                        match fs::canonicalize(&entry_path) {
                            Ok(real) => {
                                if visited_real_paths.contains(&real) {
                                    warnings.push(format!(
                                        "Symlink cycle detected, skipping: '{}'",
                                        clean_path.display()
                                    ));
                                    continue;
                                }
                                visited_real_paths.insert(real);
                            }
                            Err(e) => {
                                warnings.push(format!(
                                    "Cannot resolve symlink '{}': {}",
                                    clean_path.display(),
                                    e
                                ));
                                continue;
                            }
                        }
                    }
                }

                let mtime = get_mtime_from_metadata(&metadata);
                sub_folders.push((name, clean_path.to_path_buf(), mtime));
            } else if metadata.is_file() {
                let size = metadata.len();
                if !opts.include_zero_size && size == 0 {
                    continue;
                }
                let mtime = get_mtime_from_metadata(&metadata);
                let ext = Path::new(&name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                let est = extension_stats.entry(ext).or_insert((0, 0));
                est.0 += 1;
                est.1 += size;
                files.push((name, size, mtime));
                total_files += 1;
                total_size += size;
            }
        }

        sub_folders.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
        files.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));

        for (name, sub_path, mtime) in &sub_folders {
            let sub_id = next_id;
            next_id += 1;

            let parent_rel = &dirs[&dir_id].rel_path;
            let rel_path = if parent_rel.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", parent_rel, name)
            };

            dirs.insert(
                sub_id,
                DirEntry {
                    id: sub_id,
                    parent_id: Some(dir_id),
                    path: sub_path.clone(),
                    rel_path,
                    modified: *mtime,
                    subdirs: Vec::new(),
                    content: Vec::new(),
                },
            );
            path_to_id.insert(sub_path.clone(), sub_id);
            stack.push((sub_path.clone(), sub_id));

            if let Some(parent) = dirs.get_mut(&dir_id) {
                parent.subdirs.push(sub_id);
            }
        }

        let mut content: Vec<ContentItem> = Vec::new();
        for (name, sub_path, mtime) in &sub_folders {
            let sub_id = *path_to_id.get(sub_path).unwrap();
            content.push(ContentItem::Folder {
                title: name.clone(),
                key: sub_id,
                is_lazy: true,
                modified: *mtime,
                size: 0,
            });
        }
        for (name, size, mtime) in &files {
            content.push(ContentItem::File {
                title: name.clone(),
                size: *size,
                modified: *mtime,
            });
        }

        if let Some(dir) = dirs.get_mut(&dir_id) {
            dir.content = content;
        }

        if let Some(cb) = &progress {
            cb(&format!("Scanning... {} dirs", dirs.len()));
        }
    }

    // Post-scan: calculate sizes bottom-up
    let dir_ids: Vec<usize> = dirs.keys().copied().collect();
    let mut dir_sizes: HashMap<usize, u64> = HashMap::new();

    for &id in dir_ids.iter().rev() {
        let mut size: u64 = 0;
        for item in &dirs[&id].content {
            match item {
                ContentItem::File { size: s, .. } => size += s,
                ContentItem::Folder { key, .. } => {
                    size += dir_sizes.get(key).copied().unwrap_or(0);
                }
            }
        }
        dir_sizes.insert(id, size);
    }

    for id in dir_ids.iter().copied() {
        let subdirs_info: Vec<(usize, u64)> = dirs[&id]
            .content
            .iter()
            .filter_map(|item| {
                if let ContentItem::Folder { key, .. } = item {
                    Some((*key, dir_sizes.get(key).copied().unwrap_or(0)))
                } else {
                    None
                }
            })
            .collect();

        let has_lazy_info: Vec<(usize, bool)> = dirs[&id]
            .content
            .iter()
            .filter_map(|item| {
                if let ContentItem::Folder { key, .. } = item {
                    let has_children = dirs
                        .get(key)
                        .map(|d| !d.subdirs.is_empty())
                        .unwrap_or(false);
                    Some((*key, has_children))
                } else {
                    None
                }
            })
            .collect();

        if let Some(dir) = dirs.get_mut(&id) {
            for item in dir.content.iter_mut() {
                if let ContentItem::Folder {
                    key, size, is_lazy, ..
                } = item
                {
                    for (sid, ssize) in &subdirs_info {
                        if sid == key {
                            *size = *ssize;
                        }
                    }
                    for (sid, has_children) in &has_lazy_info {
                        if sid == key {
                            *is_lazy = *has_children;
                        }
                    }
                }
            }
        }
    }

    // Post-scan: prune empty directories
    if !opts.include_empty_dirs {
        prune_empty_dirs(&mut dirs);
    }

    let total_dirs = dirs.len() as u64;

    ScanResult {
        dirs,
        total_files,
        total_dirs,
        total_size,
        warnings,
        extension_stats,
    }
}

pub fn prune_empty_dirs(dirs: &mut BTreeMap<usize, DirEntry>) -> usize {
    let dir_ids: Vec<usize> = dirs.keys().copied().collect();
    let mut has_content: HashMap<usize, bool> = HashMap::new();

    for &id in dir_ids.iter().rev() {
        let dir = &dirs[&id];
        let has_files = dir
            .content
            .iter()
            .any(|item| matches!(item, ContentItem::File { .. }));
        let has_nonempty_subdir = dir
            .subdirs
            .iter()
            .any(|sub_id| has_content.get(sub_id).copied().unwrap_or(false));
        has_content.insert(id, has_files || has_nonempty_subdir);
    }

    let to_remove: Vec<usize> = has_content
        .iter()
        .filter(|(&id, &has)| !has && id != 0)
        .map(|(&id, _)| id)
        .collect();

    let removed_set: HashSet<usize> = to_remove.iter().copied().collect();

    for &id in &to_remove {
        dirs.remove(&id);
    }

    for dir in dirs.values_mut() {
        dir.subdirs.retain(|sub_id| !removed_set.contains(sub_id));
        dir.content.retain(|item| {
            if let ContentItem::Folder { key, .. } = item {
                !removed_set.contains(key)
            } else {
                true
            }
        });
    }

    to_remove.len()
}

// ============================================================================
// DATA STRUCTURES & COMPRESSION
// ============================================================================

pub fn build_data_structures(
    dirs: &BTreeMap<usize, DirEntry>,
    subtree_stats: &BTreeMap<usize, SubtreeStats>,
) -> (BTreeMap<usize, Map<String, Value>>, Value, Value) {
    let mut all_dir_data: BTreeMap<String, Value> = BTreeMap::new();

    for (id, dir) in dirs {
        let mut compact_content: Vec<Value> = Vec::new();
        for item in &dir.content {
            match item {
                ContentItem::Folder {
                    title,
                    key,
                    is_lazy,
                    modified,
                    size,
                } => {
                    compact_content.push(json!([
                        title,
                        key.to_string(),
                        if *is_lazy { 1 } else { 0 },
                        modified,
                        size
                    ]));
                }
                ContentItem::File {
                    title,
                    size,
                    modified,
                } => {
                    compact_content.push(json!([title, size, modified]));
                }
            }
        }
        all_dir_data.insert(id.to_string(), Value::Array(compact_content));
    }

    let sorted_keys: Vec<String> = all_dir_data.keys().cloned().collect();
    let mut chunks: BTreeMap<usize, Map<String, Value>> = BTreeMap::new();
    let mut chunk_index: Map<String, Value> = Map::new();

    for (i, key) in sorted_keys.iter().enumerate() {
        let chunk_id = i / CHUNK_SIZE;
        chunk_index.insert(key.clone(), json!(chunk_id));
        let chunk = chunks.entry(chunk_id).or_insert_with(Map::new);
        chunk.insert(key.clone(), all_dir_data[key].clone());

        // Embed subtree stats for this directory
        if let Ok(dir_id) = key.parse::<usize>() {
            if let Some(st) = subtree_stats.get(&dir_id) {
                let mut ext_vec: Vec<(&String, &(u64, u64))> =
                    st.extension_stats.iter().collect();
                ext_vec.sort_by(|a, b| b.1 .1.cmp(&a.1 .1));
                let ext_compact: Vec<Value> = ext_vec
                    .iter()
                    .map(|(ext, (count, size))| json!([ext, count, size]))
                    .collect();
                chunk.insert(
                    format!("{}_stats", key),
                    json!({
                        "tf": st.total_files,
                        "td": st.total_dirs,
                        "ts": st.total_size,
                        "ext": ext_compact
                    }),
                );
            }
        }
    }

    // Search index
    let mut search_items: Vec<Value> = Vec::new();
    for (id, dir) in dirs {
        let parent_str = dir
            .parent_id
            .map(|p| p.to_string())
            .unwrap_or_else(|| "null".to_string());

        if dir.parent_id.is_some() {
            let dir_path = &dir.rel_path;
            let dirsize: u64 = dir
                .content
                .iter()
                .map(|item| match item {
                    ContentItem::File { size, .. } => *size,
                    ContentItem::Folder { size, .. } => *size,
                })
                .sum();

            search_items.push(json!({
                "path": dir_path,
                "id": id.to_string(),
                "parent_id": parent_str,
                "is_dir": true,
                "dirsize": dirsize,
                "modified": dir.modified
            }));
        }

        for item in &dir.content {
            if let ContentItem::File {
                title,
                size,
                modified,
            } = item
            {
                let file_path = if dir.rel_path.is_empty() {
                    title.clone()
                } else {
                    format!("{}/{}", dir.rel_path, title)
                };
                search_items.push(json!({
                    "path": file_path,
                    "parent_id": id.to_string(),
                    "is_dir": false,
                    "size": size,
                    "modified": modified
                }));
            }
        }
    }

    (chunks, Value::Object(chunk_index), Value::Array(search_items))
}

pub fn compress_and_encode(data: &Value) -> Result<String, String> {
    let json_str =
        serde_json::to_string(data).map_err(|e| format!("Failed to serialize JSON: {}", e))?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder
        .write_all(json_str.as_bytes())
        .map_err(|e| format!("Failed to compress: {}", e))?;
    let compressed = encoder
        .finish()
        .map_err(|e| format!("Failed to finish compression: {}", e))?;
    Ok(BASE64.encode(&compressed))
}

pub fn compress_chunks(
    chunks: &BTreeMap<usize, Map<String, Value>>,
) -> Result<Map<String, Value>, String> {
    let mut result: Map<String, Value> = Map::new();
    for (chunk_id, chunk_data) in chunks {
        let value = Value::Object(chunk_data.clone());
        let b64 = compress_and_encode(&value)?;
        result.insert(chunk_id.to_string(), Value::String(b64));
    }
    Ok(result)
}

// ============================================================================
// STATISTICS
// ============================================================================

pub fn build_stats_json(stats: &HashMap<String, (u64, u64)>, total_files: u64, total_size: u64) -> Value {
    let mut extensions: Vec<Value> = stats
        .iter()
        .map(|(ext, (count, size))| {
            json!({
                "ext": ext,
                "count": count,
                "size": size
            })
        })
        .collect();
    extensions.sort_by(|a, b| {
        let sa = a["size"].as_u64().unwrap_or(0);
        let sb = b["size"].as_u64().unwrap_or(0);
        sb.cmp(&sa)
    });
    json!({
        "total_files": total_files,
        "total_size": total_size,
        "extensions": extensions
    })
}

// ============================================================================
// OUTPUT GENERATION
// ============================================================================

pub fn build_css(out_opts: &OutputOptions) -> Result<String, String> {
    use base64::{Engine as _, engine::general_purpose};

    // Logo: --logo overrides everything, nologo hides it, otherwise theme default
    let logo_css = if out_opts.theme == "nologo" && out_opts.logo.is_none() {
        ".app_header_icon { display: none; }".to_string()
    } else {
        let (logo_data, logo_mime, logo_w, logo_h) = if let Some(logo_path) = &out_opts.logo {
            if logo_path.exists() {
                let data = fs::read(logo_path).map_err(|e| {
                    format!("Failed to read logo file '{}': {}", logo_path.display(), e)
                })?;
                let is_png = data.starts_with(&[0x89, 0x50, 0x4E, 0x47]);
                let mime = if is_png { "image/png" } else { "image/svg+xml" };
                (data, mime, 140, 38)
            } else {
                default_logo(&out_opts.theme)
            }
        } else {
            default_logo(&out_opts.theme)
        };

        let logo_b64 = general_purpose::STANDARD.encode(&logo_data);
        format!(
            ".app_header_icon {{ display: block; width: {}px; height: {}px; background: url('data:{};base64,{}') no-repeat center/contain; }}",
            logo_w, logo_h, logo_mime, logo_b64
        )
    };

    // Theme CSS overrides (appended after main stylesheet).
    // A custom background from config wins over named themes.
    if let Some(bg) = &out_opts.header_background {
        let custom_css = format!(
            "\n/* Custom branding */\nbody, #wrapper {{ background: {}; }}\n.app_header {{ background: transparent; }}\n",
            bg
        );
        return Ok(CSS_TEMPLATE.replace("/* {{LOGO_CSS}} */", &logo_css) + &custom_css);
    }
    let theme_css = "";

    Ok(CSS_TEMPLATE.replace("/* {{LOGO_CSS}} */", &logo_css) + theme_css)
}

fn default_logo(_theme: &str) -> (Vec<u8>, &'static str, u32, u32) {
    // Default branding: the D2H mark (customizable via config branding.logoPath)
    (LOGO_SVG.to_vec(), "image/svg+xml", 140, 38)
}

fn write_default_logo(output_dir: &Path, theme: &str) -> Result<(), String> {
    if theme == "nologo" {
        return Ok(()); // No logo file needed
    }
    fs::write(output_dir.join("logo.svg"), LOGO_SVG)
        .map_err(|e| format!("Failed to write logo file: {}", e))
}

pub fn build_stats(total_files: u64, total_dirs: u64, total_size: u64) -> String {
    format!(
        "{} files in {} folders ({})",
        format_number(total_files),
        format_number(total_dirs),
        format_size(total_size)
    )
}

/// Build localized UI strings as JSON for the given language code.
pub fn build_lang_json(lang: &str) -> String {
    let obj = match lang {
        "cz" => json!({
            "name": "N\u{00e1}zev", "size": "Velikost", "modified": "Zm\u{011b}n\u{011b}no",
            "folder": "Slo\u{017e}ka", "folders": "slo\u{017e}ek", "files": "soubor\u{016f}",
            "emptyFolder": "Pr\u{00e1}zdn\u{00e1} slo\u{017e}ka",
            "loading": "Na\u{010d}\u{00ed}t\u{00e1}n\u{00ed}",
            "noResults": "\u{017d}\u{00e1}dn\u{00e9} v\u{00fd}sledky.",
            "searchBack": "Zp\u{011b}t na v\u{00fd}pis",
            "search": "Hled\u{00e1}n\u{00ed}", "searchMin": "min. 3 znaky",
            "searchBuilding": "Sestavuji vyhled\u{00e1}vac\u{00ed} index...",
            "searchError": "Chyba p\u{0159}i sestaven\u{00ed} indexu.",
            "showingResults": "Zobrazeno", "found": "Nalezeno",
            "totalFiles": "Celkem soubor\u{016f}", "totalSize": "Celkov\u{00e1} velikost",
            "fileTypes": "Typy soubor\u{016f}", "categories": "Kategorie",
            "totalFolders": "Slo\u{017e}ky", "categoriesBySize": "Kategorie dle velikosti",
            "total": "Celkem", "other": "Ostatn\u{00ed}", "noExt": "(bez p\u{0159}\u{00ed}pony)",
            "fileListing": "V\u{00fd}pis soubor\u{016f}", "folderStats": "Statistiky slo\u{017e}ky",
            "browseFolders": "Proch\u{00e1}zet slo\u{017e}ky",
            "loadError": "Chyba na\u{010d}\u{00ed}t\u{00e1}n\u{00ed} dat",
            "catPhotos": "Fotografie", "catRawPhotos": "RAW fotografie",
            "catVideo": "Video", "catAudio": "Audio",
            "catDocuments": "Dokumenty", "catArchives": "Archivy",
            "catEmail": "E-maily", "catDatabases": "Datab\u{00e1}ze",
            "catWeb": "Web", "catSourceCode": "Zdrojov\u{00e9} k\u{00f3}dy",
            "catExecutables": "Spustiteln\u{00e9} soubory",
            "tree": "Strom", "stats": "Statistiky"
        }),
        "it" => json!({
            "name": "Nome", "size": "Dimensione", "modified": "Modificato",
            "folder": "Cartella", "folders": "cartelle", "files": "file",
            "emptyFolder": "Cartella vuota",
            "loading": "Caricamento",
            "noResults": "Nessun risultato trovato.",
            "searchBack": "Torna all'elenco",
            "search": "Cerca", "searchMin": "min. 3 caratteri",
            "searchBuilding": "Creazione dell'indice di ricerca...",
            "searchError": "Errore nella creazione dell'indice.",
            "showingResults": "Visualizzati", "found": "Trovati",
            "totalFiles": "File totali", "totalSize": "Dimensione totale",
            "fileTypes": "Tipi di file", "categories": "Categorie",
            "totalFolders": "Cartelle", "categoriesBySize": "Categorie per dimensione",
            "total": "Totale", "other": "Altro", "noExt": "(senza estensione)",
            "fileListing": "Elenco file", "folderStats": "Statistiche cartella",
            "browseFolders": "Sfoglia cartelle",
            "loadError": "Errore nel caricamento dei dati",
            "catPhotos": "Fotografie", "catRawPhotos": "Foto RAW",
            "catVideo": "Video", "catAudio": "Audio",
            "catDocuments": "Documenti", "catArchives": "Archivi",
            "catEmail": "E-mail", "catDatabases": "Database",
            "catWeb": "Web", "catSourceCode": "Codice sorgente",
            "catExecutables": "Eseguibili",
            "tree": "Struttura", "stats": "Statistiche"
        }),
        "es" => json!({
            "name": "Nombre", "size": "Tama\u{f1}o", "modified": "Modificado",
            "folder": "Carpeta", "folders": "carpetas", "files": "archivos",
            "emptyFolder": "Carpeta vac\u{ed}a",
            "loading": "Cargando",
            "noResults": "No se encontraron resultados.",
            "searchBack": "Volver a la lista",
            "search": "Buscar", "searchMin": "m\u{ed}n. 3 caracteres",
            "searchBuilding": "Creando \u{ed}ndice de b\u{fa}squeda...",
            "searchError": "Error al crear el \u{ed}ndice.",
            "showingResults": "Mostrando", "found": "Encontrados",
            "totalFiles": "Archivos totales", "totalSize": "Tama\u{f1}o total",
            "fileTypes": "Tipos de archivo", "categories": "Categor\u{ed}as",
            "totalFolders": "Carpetas", "categoriesBySize": "Categor\u{ed}as por tama\u{f1}o",
            "total": "Total", "other": "Otros", "noExt": "(sin extensi\u{f3}n)",
            "fileListing": "Lista de archivos", "folderStats": "Estad\u{ed}sticas de carpeta",
            "browseFolders": "Explorar carpetas",
            "loadError": "Error al cargar los datos",
            "catPhotos": "Fotograf\u{ed}as", "catRawPhotos": "Fotos RAW",
            "catVideo": "V\u{ed}deo", "catAudio": "Audio",
            "catDocuments": "Documentos", "catArchives": "Archivos comprimidos",
            "catEmail": "Correos", "catDatabases": "Bases de datos",
            "catWeb": "Web", "catSourceCode": "C\u{f3}digo fuente",
            "catExecutables": "Ejecutables",
            "tree": "\u{c1}rbol", "stats": "Estad\u{ed}sticas"
        }),
        "fr" => json!({
            "name": "Nom", "size": "Taille", "modified": "Modifi\u{e9}",
            "folder": "Dossier", "folders": "dossiers", "files": "fichiers",
            "emptyFolder": "Dossier vide",
            "loading": "Chargement",
            "noResults": "Aucun r\u{e9}sultat trouv\u{e9}.",
            "searchBack": "Retour \u{e0} la liste",
            "search": "Rechercher", "searchMin": "min. 3 caract\u{e8}res",
            "searchBuilding": "Cr\u{e9}ation de l'index de recherche...",
            "searchError": "Erreur lors de la cr\u{e9}ation de l'index.",
            "showingResults": "Affichage", "found": "Trouv\u{e9}s",
            "totalFiles": "Fichiers au total", "totalSize": "Taille totale",
            "fileTypes": "Types de fichiers", "categories": "Cat\u{e9}gories",
            "totalFolders": "Dossiers", "categoriesBySize": "Cat\u{e9}gories par taille",
            "total": "Total", "other": "Autres", "noExt": "(sans extension)",
            "fileListing": "Liste des fichiers", "folderStats": "Statistiques du dossier",
            "browseFolders": "Parcourir les dossiers",
            "loadError": "Erreur de chargement des donn\u{e9}es",
            "catPhotos": "Photographies", "catRawPhotos": "Photos RAW",
            "catVideo": "Vid\u{e9}o", "catAudio": "Audio",
            "catDocuments": "Documents", "catArchives": "Archives",
            "catEmail": "E-mails", "catDatabases": "Bases de donn\u{e9}es",
            "catWeb": "Web", "catSourceCode": "Code source",
            "catExecutables": "Ex\u{e9}cutables",
            "tree": "Arborescence", "stats": "Statistiques"
        }),
        "pl" => json!({
            "name": "Nazwa", "size": "Rozmiar", "modified": "Zmodyfikowano",
            "folder": "Folder", "folders": "folder\u{f3}w", "files": "plik\u{f3}w",
            "emptyFolder": "Pusty folder",
            "loading": "\u{141}adowanie",
            "noResults": "Nie znaleziono wynik\u{f3}w.",
            "searchBack": "Powr\u{f3}t do listy",
            "search": "Szukaj", "searchMin": "min. 3 znaki",
            "searchBuilding": "Tworzenie indeksu wyszukiwania...",
            "searchError": "B\u{142}\u{105}d podczas tworzenia indeksu.",
            "showingResults": "Wy\u{15b}wietlono", "found": "Znaleziono",
            "totalFiles": "\u{141}\u{105}cznie plik\u{f3}w", "totalSize": "Ca\u{142}kowity rozmiar",
            "fileTypes": "Typy plik\u{f3}w", "categories": "Kategorie",
            "totalFolders": "Foldery", "categoriesBySize": "Kategorie wed\u{142}ug rozmiaru",
            "total": "\u{141}\u{105}cznie", "other": "Inne", "noExt": "(bez rozszerzenia)",
            "fileListing": "Lista plik\u{f3}w", "folderStats": "Statystyki folderu",
            "browseFolders": "Przegl\u{105}daj foldery",
            "loadError": "B\u{142}\u{105}d \u{142}adowania danych",
            "catPhotos": "Zdj\u{119}cia", "catRawPhotos": "Zdj\u{119}cia RAW",
            "catVideo": "Wideo", "catAudio": "Audio",
            "catDocuments": "Dokumenty", "catArchives": "Archiwa",
            "catEmail": "E-maile", "catDatabases": "Bazy danych",
            "catWeb": "Web", "catSourceCode": "Kod \u{17a}r\u{f3}d\u{142}owy",
            "catExecutables": "Pliki wykonywalne",
            "tree": "Drzewo", "stats": "Statystyki"
        }),
        "de" => json!({
            "name": "Name", "size": "Gr\u{00f6}\u{00df}e", "modified": "Ge\u{00e4}ndert",
            "folder": "Ordner", "folders": "Ordner", "files": "Dateien",
            "emptyFolder": "Leerer Ordner",
            "loading": "Laden",
            "noResults": "Keine Ergebnisse gefunden.",
            "searchBack": "Zur\u{00fc}ck zur Liste",
            "search": "Suche", "searchMin": "min. 3 Zeichen",
            "searchBuilding": "Suchindex wird erstellt...",
            "searchError": "Fehler beim Erstellen des Suchindex.",
            "showingResults": "Anzeige", "found": "Gefunden",
            "totalFiles": "Dateien gesamt", "totalSize": "Gesamtgr\u{00f6}\u{00df}e",
            "fileTypes": "Dateitypen", "categories": "Kategorien",
            "totalFolders": "Ordner", "categoriesBySize": "Kategorien nach Gr\u{00f6}\u{00df}e",
            "total": "Gesamt", "other": "Sonstige", "noExt": "(ohne Erw.)",
            "fileListing": "Dateiliste", "folderStats": "Ordnerstatistik",
            "browseFolders": "Ordnerstruktur",
            "loadError": "Fehler beim Laden der Daten",
            "catPhotos": "Fotos", "catRawPhotos": "RAW-Fotos",
            "catVideo": "Video", "catAudio": "Audio",
            "catDocuments": "Dokumente", "catArchives": "Archive",
            "catEmail": "E-Mails", "catDatabases": "Datenbanken",
            "catWeb": "Web", "catSourceCode": "Quellcode",
            "catExecutables": "Ausf\u{00fc}hrbar",
            "tree": "Struktur", "stats": "Statistik"
        }),
        _ => json!({
            "name": "Name", "size": "Size", "modified": "Modified",
            "folder": "Folder", "folders": "folders", "files": "files",
            "emptyFolder": "Empty folder",
            "loading": "Loading",
            "noResults": "No results found.",
            "searchBack": "Back to listing",
            "search": "Search", "searchMin": "min. 3 chars",
            "searchBuilding": "Building search index...",
            "searchError": "Error preparing search index.",
            "showingResults": "Showing", "found": "Found",
            "totalFiles": "Total Files", "totalSize": "Total Size",
            "fileTypes": "File Types", "categories": "Categories",
            "totalFolders": "Folders", "categoriesBySize": "Categories by Size",
            "total": "Total", "other": "Other", "noExt": "(no ext)",
            "fileListing": "File listing", "folderStats": "Folder statistics",
            "browseFolders": "Browse folders",
            "loadError": "Error loading data",
            "catPhotos": "Photographs", "catRawPhotos": "RAW Photos",
            "catVideo": "Video", "catAudio": "Audio",
            "catDocuments": "Documents", "catArchives": "Archives",
            "catEmail": "Email", "catDatabases": "Databases",
            "catWeb": "Web", "catSourceCode": "Source Code",
            "catExecutables": "Executables",
            "tree": "Tree", "stats": "Stats"
        }),
    };
    serde_json::to_string(&obj).unwrap()
}

pub fn write_json_gz(data: &Value, path: &Path) -> Result<(), String> {
    let json_str =
        serde_json::to_string(data).map_err(|e| format!("Failed to serialize JSON: {}", e))?;
    let file = fs::File::create(path)
        .map_err(|e| format!("Failed to create '{}': {}", path.display(), e))?;
    let mut encoder = GzEncoder::new(file, Compression::best());
    encoder
        .write_all(json_str.as_bytes())
        .map_err(|e| format!("Failed to write '{}': {}", path.display(), e))?;
    encoder
        .finish()
        .map_err(|e| format!("Failed to finish '{}': {}", path.display(), e))?;
    Ok(())
}

pub fn generate_multi_file_output(
    output_dir: &Path,
    title: &str,
    total_files: u64,
    total_dirs: u64,
    total_size: u64,
    chunks: &BTreeMap<usize, Map<String, Value>>,
    chunk_index: &Value,
    search_index: &Value,
    stats_json: &Value,
    out_opts: &OutputOptions,
    progress: Option<&dyn Fn(&str)>,
) -> Result<(), String> {
    let data_dir = output_dir.join("data");
    fs::create_dir_all(&data_dir)
        .map_err(|e| format!("Failed to create data directory: {}", e))?;

    for (i, (chunk_id, chunk_data)) in chunks.iter().enumerate() {
        let value = Value::Object(chunk_data.clone());
        let gz_path = data_dir.join(format!("chunk_{}.json.gz", chunk_id));
        write_json_gz(&value, &gz_path)?;

        if let Some(cb) = &progress {
            cb(&format!("Writing chunk {}/{}", i + 1, chunks.len()));
        }
    }

    // Chunk index
    let idx_gz_path = data_dir.join("chunk_index.json.gz");
    write_json_gz(chunk_index, &idx_gz_path)?;

    // Search index
    let search_gz_path = output_dir.join("full_index.json.gz");
    write_json_gz(search_index, &search_gz_path)?;

    // Write assets (from override or embedded default)
    if let Some(pako_path) = &out_opts.pako {
        if pako_path.exists() {
            fs::copy(pako_path, output_dir.join("pako.min.js"))
                .map_err(|e| format!("Failed to copy pako.min.js: {}", e))?;
        } else {
            fs::write(output_dir.join("pako.min.js"), PAKO_JS)
                .map_err(|e| format!("Failed to write embedded pako.min.js: {}", e))?;
        }
    } else {
        fs::write(output_dir.join("pako.min.js"), PAKO_JS)
            .map_err(|e| format!("Failed to write embedded pako.min.js: {}", e))?;
    }
    // Write logo file (respects --logo override, then theme default)
    if let Some(logo_path) = &out_opts.logo {
        if logo_path.exists() {
            let ext = logo_path.extension().and_then(|e| e.to_str()).unwrap_or("svg");
            fs::copy(logo_path, output_dir.join(format!("logo.{}", ext)))
                .map_err(|e| format!("Failed to copy logo: {}", e))?;
        } else {
            write_default_logo(output_dir, &out_opts.theme)?;
        }
    } else {
        write_default_logo(output_dir, &out_opts.theme)?;
    }

    // Generate index.html
    let css = build_css(out_opts)?;
    let stats = build_stats(total_files, total_dirs, total_size);

    let lang_json = build_lang_json(&out_opts.lang);
    let js_noinit = JS_TEMPLATE
        .replace("{{ROOT_NAME}}", &js_escape(title))
        .replace("// {{LANG_JSON}}", &format!("var LANG = {};", lang_json))
        .replace(
            "// {{AUTO_INIT}}",
            "// Auto-init disabled — external loader handles initialization",
        );

    // Escape '<' inside inline JSON so file-derived strings cannot break out
    // of the surrounding <script> element ("<" is a valid JSON escape).
    let stats_inline = serde_json::to_string(stats_json)
        .unwrap_or_else(|_| "null".to_string())
        .replace('<', "\\u003c");

    let lang_obj: serde_json::Value = serde_json::from_str(&lang_json)
        .map_err(|e| format!("Failed to parse language JSON: {}", e))?;
    let l = |key: &str| -> String {
        lang_obj.get(key).and_then(|v| v.as_str()).unwrap_or(key).to_string()
    };

    let html = MULTI_HTML_TEMPLATE
        .replace("{{TITLE}}", &html_escape(title))
        .replace("{{CSS}}", &css)
        .replace("{{FOOTER_HTML}}", &out_opts.footer_html)
        .replace("{{STATS}}", &stats)
        .replace("{{STATS_INLINE}}", &stats_inline)
        .replace("{{JS_NOINIT}}", &js_noinit)
        .replace("{{EXTERNAL_LOADER}}", EXTERNAL_LOADER_JS)
        .replace("{{LABEL_TREE}}", &l("tree"))
        .replace("{{LABEL_STATS}}", &l("stats"))
        .replace("{{LABEL_FILE_LISTING}}", &l("fileListing"))
        .replace("{{LABEL_FOLDER_STATS}}", &l("folderStats"))
        .replace("{{LABEL_SEARCH}}", &l("search"))
        .replace("{{LABEL_SEARCH_MIN}}", &l("searchMin"))
        .replace("{{LABEL_BROWSE_FOLDERS}}", &l("browseFolders"))
        .replace("{{LABEL_LOADING}}", &l("loading"));

    fs::write(output_dir.join("index.html"), html)
        .map_err(|e| format!("Failed to write index.html: {}", e))?;
    Ok(())
}

pub fn generate_single_file_html(
    title: &str,
    total_files: u64,
    total_dirs: u64,
    total_size: u64,
    compressed_index: &str,
    compressed_chunks: &Map<String, Value>,
    compressed_search: &str,
    stats_json: &Value,
    out_opts: &OutputOptions,
) -> Result<String, String> {
    let css = build_css(out_opts)?;
    let stats = build_stats(total_files, total_dirs, total_size);

    let lang_json = build_lang_json(&out_opts.lang);
    let js = JS_TEMPLATE
        .replace("{{ROOT_NAME}}", &js_escape(title))
        .replace("// {{LANG_JSON}}", &format!("var LANG = {};", lang_json))
        .replace(
            "// {{AUTO_INIT}}",
            "document.addEventListener(\"DOMContentLoaded\", init);",
        );

    let pako_content = if let Some(pako_path) = &out_opts.pako {
        if pako_path.exists() {
            fs::read_to_string(pako_path)
                .map_err(|e| format!("Failed to read pako.min.js: {}", e))?
        } else {
            PAKO_JS.to_string()
        }
    } else {
        PAKO_JS.to_string()
    };
    let pako_script = format!("<script>/* pako.min.js embedded */\n{}</script>", pako_content);

    let chunks_json = serde_json::to_string(compressed_chunks)
        .map_err(|e| format!("Failed to serialize chunks: {}", e))?;

    // Escape '<' inside inline JSON — see generate_multi_file_output.
    let stats_inline = serde_json::to_string(stats_json)
        .unwrap_or_else(|_| "null".to_string())
        .replace('<', "\\u003c");

    let lang_obj: serde_json::Value = serde_json::from_str(&lang_json)
        .map_err(|e| format!("Failed to parse language JSON: {}", e))?;
    let l = |key: &str| -> String {
        lang_obj.get(key).and_then(|v| v.as_str()).unwrap_or(key).to_string()
    };

    Ok(HTML_TEMPLATE
        .replace("{{TITLE}}", &html_escape(title))
        .replace("{{CSS}}", &css)
        .replace("{{FOOTER_HTML}}", &out_opts.footer_html)
        .replace("{{STATS}}", &stats)
        .replace("{{STATS_INLINE}}", &stats_inline)
        .replace("{{PAKO_SCRIPT}}", &pako_script)
        .replace("{{INDEX_B64}}", compressed_index)
        .replace("{{CHUNKS_B64}}", &chunks_json)
        .replace("{{SEARCH_B64}}", compressed_search)
        .replace("{{JS}}", &js)
        .replace("{{LABEL_TREE}}", &l("tree"))
        .replace("{{LABEL_STATS}}", &l("stats"))
        .replace("{{LABEL_FILE_LISTING}}", &l("fileListing"))
        .replace("{{LABEL_FOLDER_STATS}}", &l("folderStats"))
        .replace("{{LABEL_SEARCH}}", &l("search"))
        .replace("{{LABEL_SEARCH_MIN}}", &l("searchMin"))
        .replace("{{LABEL_BROWSE_FOLDERS}}", &l("browseFolders"))
        .replace("{{LABEL_LOADING}}", &l("loading")))
}

// ============================================================================
// PLATFORM-SPECIFIC HELPERS
// ============================================================================

#[cfg(target_os = "windows")]
pub fn long_path(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if s.starts_with("\\\\?\\") {
        return path.to_path_buf();
    }
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_default()
            .join(path)
    };
    let abs_str = abs.to_string_lossy();
    if abs_str.starts_with("\\\\") {
        PathBuf::from(format!("\\\\?\\UNC\\{}", &abs_str[2..]))
    } else {
        PathBuf::from(format!("\\\\?\\{}", abs_str))
    }
}

#[cfg(not(target_os = "windows"))]
pub fn long_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}

#[cfg(target_os = "windows")]
pub fn strip_long_path(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if s.starts_with("\\\\?\\UNC\\") {
        PathBuf::from(format!("\\\\{}", &s[8..]))
    } else if s.starts_with("\\\\?\\") {
        PathBuf::from(&s[4..])
    } else {
        path.to_path_buf()
    }
}

#[cfg(not(target_os = "windows"))]
pub fn strip_long_path(path: &Path) -> PathBuf {
    path.to_path_buf()
}

#[cfg(target_os = "windows")]
fn is_hidden_windows(path: &Path, warnings: &mut Vec<String>) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;

    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            let attrs = metadata.file_attributes();
            // Both files and directories: skip if Hidden or System attribute is set
            // System Volume Information, $Recycle.Bin etc. have SYSTEM and/or HIDDEN
            (attrs & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM)) != 0
        }
        Err(e) => {
            warnings.push(format!(
                "Cannot read attributes of '{}': {}",
                path.display(),
                e
            ));
            false
        }
    }
}

fn is_macos_package(name: &str) -> bool {
    let lower = name.to_lowercase();
    PACKAGE_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

fn calculate_dir_size(path: &Path, warnings: &mut Vec<String>) -> u64 {
    let mut total: u64 = 0;
    let scan_path = long_path(path);
    let mut stack = vec![scan_path];

    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                warnings.push(format!(
                    "Cannot read package dir '{}': {}",
                    dir.display(),
                    e
                ));
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            match fs::symlink_metadata(entry.path()) {
                Ok(m) => {
                    if m.is_file() {
                        total += m.len();
                    } else if m.is_dir() {
                        stack.push(entry.path());
                    }
                }
                Err(_) => continue,
            }
        }
    }
    total
}

// ============================================================================
// FORMATTING HELPERS
// ============================================================================

fn get_mtime(path: &Path) -> i64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn get_mtime_from_metadata(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn format_size(bytes: u64) -> String {
    if bytes == 0 {
        return "0 B".to_string();
    }
    let units = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut size = bytes as f64;
    for unit in &units {
        if size < 1024.0 {
            return if *unit == "B" {
                format!("{} B", size as u64)
            } else {
                format!("{:.1} {}", size, unit)
            };
        }
        size /= 1024.0;
    }
    format!("{:.1} PB", size)
}

pub fn format_number(n: u64) -> String {
    let s = n.to_string();
    let mut result = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    result.chars().rev().collect()
}

pub fn js_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        // '<' must be escaped: a literal "</script>" inside an inline script
        // string terminates the script element regardless of JS quoting.
        .replace('<', "\\x3c")
        // Line/paragraph separators are valid JSON but illegal in pre-ES2019 JS strings.
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn now_iso8601() -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let secs_per_day: u64 = 86400;
    let secs_per_hour: u64 = 3600;
    let secs_per_min: u64 = 60;

    let days = now / secs_per_day;
    let time_of_day = now % secs_per_day;
    let hours = time_of_day / secs_per_hour;
    let minutes = (time_of_day % secs_per_hour) / secs_per_min;
    let seconds = time_of_day % secs_per_min;

    let mut y: i64 = 1970;
    let mut remaining_days = days as i64;

    loop {
        let days_in_year = if is_leap_year(y) { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        y += 1;
    }

    let month_days = if is_leap_year(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut m: usize = 0;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining_days < md as i64 {
            m = i + 1;
            break;
        }
        remaining_days -= md as i64;
    }
    let d = remaining_days + 1;

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y, m, d, hours, minutes, seconds
    )
}

fn is_leap_year(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn dir_total_size(path: &Path) -> u64 {
    let mut total: u64 = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if meta.is_file() {
                        total += meta.len();
                    } else if meta.is_dir() {
                        stack.push(entry.path());
                    }
                }
            }
        }
    }
    total
}

/// Create a temporary workspace directory for generating web files.
/// Uses system temp with unique name: d2h_temp_{PID}_{timestamp}.
/// If an old temp dir exists (from a crash), it is cleaned up first.
pub fn create_temp_workspace() -> Result<PathBuf, String> {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let temp_name = format!("d2h_temp_{}_{}", std::process::id(), now);
    let temp_dir = std::env::temp_dir().join(temp_name);

    // Clean up any old d2h_temp_* dirs (from crashes) older than 1 hour
    if let Ok(entries) = fs::read_dir(std::env::temp_dir()) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("d2h_temp_") && entry.path() != temp_dir {
                if let Ok(meta) = entry.metadata() {
                    if let Ok(modified) = meta.modified() {
                        let age = SystemTime::now()
                            .duration_since(modified)
                            .unwrap_or_default()
                            .as_secs();
                        if age > 3600 {
                            let _ = fs::remove_dir_all(entry.path());
                        }
                    }
                }
            }
        }
    }

    if temp_dir.exists() {
        let _ = fs::remove_dir_all(&temp_dir);
    }
    fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("Failed to create temp workspace: {}", e))?;

    Ok(temp_dir)
}

/// Remove the temporary workspace directory.
pub fn remove_temp_workspace(temp_dir: &Path) {
    if temp_dir.exists() {
        let _ = fs::remove_dir_all(temp_dir);
    }
}

/// Create a .tar.gz archive from a source directory (temp workspace).
/// The archive is written to a temp file first, then moved to final_path.
pub fn create_tar_gz(source_dir: &Path, final_path: &Path) -> Result<(), String> {
    // Append ".tmp" instead of with_extension() — with_extension would replace
    // the ".gz" part and produce "name.tar.tar.gz.tmp"-style names.
    let mut temp_os = final_path.as_os_str().to_os_string();
    temp_os.push(".tmp");
    let temp_path = PathBuf::from(temp_os);

    let build = (|| -> Result<(), String> {
        let file = fs::File::create(&temp_path)
            .map_err(|e| format!("Failed to create temp tar.gz: {}", e))?;
        let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut tar = tar::Builder::new(enc);

        let mut stack = vec![source_dir.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let mut entries: Vec<_> = fs::read_dir(&dir)
                .map_err(|e| format!("Failed to read dir: {}", e))?
                .flatten()
                .collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                let path = entry.path();
                let rel = path
                    .strip_prefix(source_dir)
                    .map_err(|e| format!("Failed to compute relative path: {}", e))?
                    .to_path_buf();
                if path.is_file() {
                    tar.append_path_with_name(&path, &rel)
                        .map_err(|e| format!("Failed to add file to tar: {}", e))?;
                } else if path.is_dir() {
                    tar.append_dir(&rel, &path)
                        .map_err(|e| format!("Failed to add dir to tar: {}", e))?;
                    stack.push(path);
                }
            }
        }

        // Explicitly finish BOTH the tar stream and the gzip encoder.
        // Without GzEncoder::finish() the gzip footer is written in Drop,
        // where I/O errors (full disk, dropped network share) are silently
        // swallowed — producing a corrupt archive reported as success.
        let enc = tar
            .into_inner()
            .map_err(|e| format!("Failed to finish tar: {}", e))?;
        enc.finish()
            .map_err(|e| format!("Failed to finish gzip compression: {}", e))?;
        Ok(())
    })();

    // On any build error, clean up the temp file instead of leaving it behind.
    if let Err(e) = build {
        let _ = fs::remove_file(&temp_path);
        return Err(e);
    }

    fs::rename(&temp_path, final_path)
        .or_else(|_| {
            fs::copy(&temp_path, final_path)
                .and_then(|_| fs::remove_file(&temp_path))
        })
        .map_err(|e| {
            let _ = fs::remove_file(&temp_path);
            format!("Failed to move tar.gz to output: {}", e)
        })?;

    Ok(())
}

/// Find a unique filename in output_dir. If {name}.{ext} exists,
/// tries {name}-1.{ext}, {name}-2.{ext}, etc.
/// Returns the final base name (without extension).
pub fn unique_name(output_dir: &Path, base_name: &str, extensions: &[&str]) -> String {
    // Check if any extension variant exists
    let any_exists = |name: &str| -> bool {
        extensions.iter().any(|ext| output_dir.join(format!("{}.{}", name, ext)).exists())
    };

    if !any_exists(base_name) {
        return base_name.to_string();
    }

    for i in 1..10000 {
        let candidate = format!("{}-{}", base_name, i);
        if !any_exists(&candidate) {
            return candidate;
        }
    }

    // Extremely unlikely fallback
    format!("{}-{}", base_name, std::process::id())
}

/// Canonicalize a path for comparison purposes. If the path itself does not
/// exist yet (typical for output dirs created later), canonicalize the nearest
/// existing ancestor and re-append the remainder. Windows verbatim prefixes
/// (`\\?\`, `\\?\UNC\`) are stripped so string/prefix comparisons work.
fn canonicalize_lenient(path: &Path) -> PathBuf {
    if let Ok(p) = fs::canonicalize(path) {
        return strip_long_path(&p);
    }
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = path.to_path_buf();
    while let (Some(parent), Some(name)) = (
        cur.parent().map(|p| p.to_path_buf()),
        cur.file_name().map(|n| n.to_os_string()),
    ) {
        rest.push(name);
        cur = parent;
        if let Ok(p) = fs::canonicalize(&cur) {
            let mut out = strip_long_path(&p);
            for c in rest.iter().rev() {
                out.push(c);
            }
            return out;
        }
    }
    path.to_path_buf()
}

/// Check if output_dir is safe to use (not the same as source, not a parent of source,
/// not a system directory).
pub fn validate_output_dir(source_dir: &Path, output_dir: &Path) -> Result<(), String> {
    let source = canonicalize_lenient(source_dir);
    let output = canonicalize_lenient(output_dir);

    // Windows filesystems are case-insensitive — compare lowercased paths.
    // Path-based (component-wise) starts_with avoids false prefix matches
    // like "C:\Web" vs "C:\Web_Archive".
    #[cfg(target_os = "windows")]
    let (source_cmp, output_cmp) = (
        PathBuf::from(source.to_string_lossy().replace('/', "\\").to_lowercase()),
        PathBuf::from(output.to_string_lossy().replace('/', "\\").to_lowercase()),
    );
    #[cfg(not(target_os = "windows"))]
    let (source_cmp, output_cmp) = (source.clone(), output.clone());

    // Same directory
    if source_cmp == output_cmp {
        return Err("Output directory cannot be the same as source directory.".to_string());
    }

    // Output is parent of source (source is inside output)
    if source_cmp.starts_with(&output_cmp) {
        return Err(format!(
            "Output directory '{}' is a parent of the source directory. This would include source files in the output.",
            output_dir.display()
        ));
    }

    // System directories protection
    let output_str = output.to_string_lossy().to_string();
    #[cfg(not(target_os = "windows"))]
    {
        let dangerous = ["/", "/Users", "/home", "/tmp", "/var", "/etc", "/System", "/Applications"];
        for d in &dangerous {
            if output_str == *d {
                return Err(format!("Cannot use system directory '{}' as output.", d));
            }
        }
        // Home directory root
        if let Ok(home) = std::env::var("HOME") {
            if output_str == home {
                return Err("Cannot use home directory as output.".to_string());
            }
            // Desktop, Documents, Downloads
            let protected = ["Desktop", "Documents", "Downloads"];
            for p in &protected {
                if output_str == format!("{}/{}", home, p) {
                    return Err(format!(
                        "Cannot use ~/{} as output. Use a subdirectory instead.",
                        p
                    ));
                }
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        let normalized = output_str.replace('/', "\\").to_lowercase();
        let trimmed = normalized.trim_end_matches('\\');
        // Drive roots: "c:" / "c:\"
        if trimmed.len() == 2 && trimmed.ends_with(':') {
            return Err(format!("Cannot use drive root '{}' as output.", output_str));
        }
        if let Ok(profile) = std::env::var("USERPROFILE") {
            let profile_lower = profile.replace('/', "\\").to_lowercase();
            let profile_trimmed = profile_lower.trim_end_matches('\\');
            if trimmed == profile_trimmed {
                return Err("Cannot use user profile directory as output.".to_string());
            }
            let protected = ["desktop", "documents", "downloads"];
            for p in &protected {
                if trimmed == format!("{}\\{}", profile_trimmed, p) {
                    return Err(format!(
                        "Cannot use {} as output. Use a subdirectory instead.",
                        p
                    ));
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generic test config — any site-specific patterns live only in the
    /// (private) deployment config, never in code or tests.
    fn test_config() -> Config {
        let mut c = Config::default();
        c.job_id.pattern = Some(r"\d{5}".to_string());
        c.job_id.warn_pattern = Some(r"\d+".to_string());
        c
    }

    #[test]
    fn valid_job_id_per_pattern() {
        let cfg = test_config();
        assert!(is_valid_job_id("57345", &cfg));
        assert!(is_valid_job_id("10000", &cfg));
        assert!(!is_valid_job_id("1234", &cfg)); // too short
        assert!(!is_valid_job_id("123456", &cfg)); // too long (anchored)
        assert!(!is_valid_job_id("abc", &cfg));
        assert!(!is_valid_job_id("", &cfg));
        // No pattern configured (public default) → nothing is a job id
        assert!(!is_valid_job_id("57345", &Config::default()));
    }

    #[test]
    fn warn_pattern_detects_lookalikes() {
        let cfg = test_config();
        assert!(looks_like_invalid_job_id("1234", &cfg)); // numeric but invalid
        assert!(!looks_like_invalid_job_id("57345", &cfg)); // valid → no warning
        assert!(!looks_like_invalid_job_id("my-backup", &cfg));
        assert!(!looks_like_invalid_job_id("1234", &Config::default()));
    }

    #[test]
    fn extract_job_id_exact_and_prefix() {
        let cfg = test_config();
        assert_eq!(extract_job_id("57345", &cfg), Some(("57345".to_string(), true)));
        assert_eq!(extract_job_id("57345-data", &cfg), Some(("57345".to_string(), false)));
        assert_eq!(extract_job_id("573456", &cfg), None); // digit right after match
        assert_eq!(extract_job_id("my-backup", &cfg), None);
        assert_eq!(extract_job_id("57345", &Config::default()), None);
    }

    #[test]
    fn rule_matching_first_wins() {
        let mut cfg = test_config();
        cfg.job_id.rules = vec![
            JobIdRule { r#match: r"^9".to_string(), language: Some("de".to_string()), ..Default::default() },
            JobIdRule { r#match: r"^\d".to_string(), language: Some("en".to_string()), ..Default::default() },
        ];
        assert_eq!(find_matching_rule("91234", &cfg).unwrap().language.as_deref(), Some("de"));
        assert_eq!(find_matching_rule("51234", &cfg).unwrap().language.as_deref(), Some("en"));
        assert!(find_matching_rule("abc", &cfg).is_none());
    }

    #[test]
    fn all_languages_have_same_keys() {
        let reference: serde_json::Value =
            serde_json::from_str(&build_lang_json("en")).unwrap();
        let ref_keys: std::collections::BTreeSet<&str> =
            reference.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        assert!(!ref_keys.is_empty());
        for lang in ["cz", "de", "it", "es", "fr", "pl"] {
            let v: serde_json::Value =
                serde_json::from_str(&build_lang_json(lang)).unwrap();
            let keys: std::collections::BTreeSet<&str> =
                v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
            assert_eq!(keys, ref_keys, "language '{}' key set differs from 'en'", lang);
        }
        // Unknown language falls back to English
        assert_eq!(build_lang_json("xx"), build_lang_json("en"));
    }

    #[test]
    fn footer_html_rendering() {
        let cfg = Config::default();
        let footer = build_footer_html(&cfg);
        assert!(footer.contains("datahelp.eu"));
        assert!(footer.contains("Generated with D2H"));

        let mut hidden = Config::default();
        hidden.branding.show_footer = false;
        assert_eq!(build_footer_html(&hidden), "");

        let mut custom = Config::default();
        custom.branding.footer_text = "ACME <Recovery>".to_string();
        custom.branding.footer_url = String::new();
        let f = build_footer_html(&custom);
        assert!(f.contains("&lt;Recovery&gt;")); // escaped
        assert!(!f.contains("<a "));
    }

    #[test]
    fn js_escape_blocks_script_breakout() {
        let escaped = js_escape("</script><img src=x>");
        assert!(!escaped.contains('<'));
        assert_eq!(js_escape("a'b\"c"), "a\\'b\\\"c");
    }

    #[test]
    fn html_escape_all_specials() {
        assert_eq!(
            html_escape("<a href=\"x\">&'</a>"),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;&lt;/a&gt;"
        );
    }

    #[test]
    fn format_size_units() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
    }

    #[test]
    fn format_number_thousands() {
        assert_eq!(format_number(1234567), "1,234,567");
    }
}
