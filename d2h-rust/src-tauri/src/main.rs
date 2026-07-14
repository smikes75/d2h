#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use d2h::*;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::State;

// ============================================================================
// WEBVIEW2 ERROR DIALOG (Windows only)
// ============================================================================

#[cfg(target_os = "windows")]
fn show_error_dialog(caption: &str, text: &str) {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(hwnd: isize, text: *const u16, caption: *const u16, utype: u32) -> i32;
    }

    fn to_wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    let caption_w = to_wide(caption);
    let text_w = to_wide(text);

    unsafe {
        MessageBoxW(0, text_w.as_ptr(), caption_w.as_ptr(), 0x00000010);
    }
}

// ============================================================================
// FRONTEND DIALOG SYSTEM (cross-platform)
// ============================================================================

#[derive(Clone, Serialize)]
struct DialogButton {
    id: String,
    label: String,
    style: String, // "primary", "warning", "default"
}

#[derive(Clone, Serialize)]
struct DialogInput {
    default_value: String,
    label: String,
}

#[derive(Clone, Serialize)]
struct DialogQuestion {
    id: String,
    title: String,
    message: String,
    buttons: Vec<DialogButton>,
    input: Option<DialogInput>,
    /// Raw values for frontend-side localization (the frontend translates
    /// title/message/button labels by `id` and interpolates these params;
    /// title/message above are the English fallback).
    params: Vec<String>,
}

/// How long to wait for the user to answer a dialog before giving up.
const DIALOG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Ask user a question via frontend modal dialog.
/// Sets progress.question, blocks until frontend calls answer_dialog
/// (or DIALOG_TIMEOUT elapses — prevents a generation thread hanging forever
/// if the frontend never answers).
fn ask_user(
    progress: &Arc<Mutex<ProgressInfo>>,
    rx: &std::sync::mpsc::Receiver<String>,
    question: DialogQuestion,
) -> Result<String, String> {
    // Drain any stale answers left over from a previous dialog
    // (e.g. a re-shown modal answered twice).
    while rx.try_recv().is_ok() {}
    {
        let mut p = progress.lock().unwrap();
        p.question = Some(question);
    }
    let answer = rx.recv_timeout(DIALOG_TIMEOUT);
    {
        let mut p = progress.lock().unwrap();
        p.question = None;
    }
    answer.map_err(|_| "Dialog was not answered (timeout or channel closed).".to_string())
}

/// Parse an answer coming from an input dialog. The frontend prefixes typed
/// values with "input:" so a user literally typing "cancel" is not mistaken
/// for the Cancel button. Returns None for cancel/empty.
fn parse_input_answer(answer: &str) -> Option<String> {
    match answer.strip_prefix("input:") {
        Some(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
        _ => None,
    }
}

// ============================================================================
// FILE COLLISION DIALOG
// ============================================================================

/// Resolve output name: if file exists, ask user to overwrite or rename.
fn resolve_output_name(
    dest: &std::path::Path,
    base_name: &str,
    extensions: &[&str],
    progress: &Arc<Mutex<ProgressInfo>>,
    rx: &std::sync::mpsc::Receiver<String>,
) -> Result<String, String> {
    let ext = extensions.join(".");
    let file_path = dest.join(format!("{}.{}", base_name, ext));

    if !file_path.exists() {
        return Ok(base_name.to_string());
    }

    // Dialog 1: Overwrite / Rename / Cancel
    let answer = ask_user(progress, rx, DialogQuestion {
        id: "file_exists".into(),
        title: "File Exists".into(),
        message: format!(
            "{}.{} already exists in {}\n\nOverwrite existing file or rename?",
            base_name, ext, dest.display()
        ),
        buttons: vec![
            DialogButton { id: "overwrite".into(), label: "Overwrite".into(), style: "warning".into() },
            DialogButton { id: "rename".into(), label: "Rename".into(), style: "primary".into() },
            DialogButton { id: "cancel".into(), label: "Cancel".into(), style: "default".into() },
        ],
        input: None,
        params: vec![format!("{}.{}", base_name, ext), dest.display().to_string()],
    })?;

    match answer.as_str() {
        "overwrite" => return Ok(base_name.to_string()),
        "rename" => {}
        _ => return Err("Generation cancelled by user.".to_string()),
    }

    // Dialog 2: Enter new name (pre-filled with suggested unique name)
    let suggested = unique_name(dest, base_name, extensions);
    let new_name = ask_user(progress, rx, DialogQuestion {
        id: "rename_input".into(),
        title: "Rename".into(),
        message: format!("Enter new filename (without .{}):", ext),
        buttons: vec![
            DialogButton { id: "ok".into(), label: "OK".into(), style: "primary".into() },
            DialogButton { id: "cancel".into(), label: "Cancel".into(), style: "default".into() },
        ],
        input: Some(DialogInput {
            default_value: suggested,
            label: "Filename".into(),
        }),
        params: vec![ext.clone()],
    })?;

    match parse_input_answer(&new_name) {
        Some(name) => Ok(name),
        None => Err("Generation cancelled by user.".to_string()),
    }
}

// ============================================================================
// JOB NUMBER VALIDATION
// ============================================================================
// Detection rules come from d2h.config.json (jobId.pattern / warnPattern) —
// without a config nothing is detected and this dialog never appears.

fn ask_invalid_case_name(
    file_name: &str,
    format_hint: &str,
    progress: &Arc<Mutex<ProgressInfo>>,
    rx: &std::sync::mpsc::Receiver<String>,
) -> Result<Option<String>, String> {
    let answer = ask_user(progress, rx, DialogQuestion {
        id: "invalid_case".into(),
        title: "Invalid Job Number".into(),
        message: format!(
            "'{}' is not a valid job number.\n\n\
             Expected: {}\n\n\
             Rename or keep current name?",
            file_name, format_hint
        ),
        buttons: vec![
            DialogButton { id: "rename".into(), label: "Rename".into(), style: "primary".into() },
            DialogButton { id: "keep".into(), label: "Keep".into(), style: "default".into() },
            DialogButton { id: "cancel".into(), label: "Cancel".into(), style: "default".into() },
        ],
        input: None,
        params: vec![file_name.to_string(), format_hint.to_string()],
    })?;
    match answer.as_str() {
        "rename" => {
            // Show input dialog for new name
            let new_name = ask_user(progress, rx, DialogQuestion {
                id: "rename_case_input".into(),
                title: "Rename".into(),
                message: "Enter new case number:".into(),
                buttons: vec![
                    DialogButton { id: "ok".into(), label: "OK".into(), style: "primary".into() },
                    DialogButton { id: "cancel".into(), label: "Cancel".into(), style: "default".into() },
                ],
                input: Some(DialogInput {
                    default_value: file_name.to_string(),
                    label: "Job number".into(),
                }),
                params: vec![],
            })?;
            match parse_input_answer(&new_name) {
                Some(name) => Ok(Some(name)),
                None => Err("Generation cancelled by user.".to_string()),
            }
        }
        "keep" => Ok(None),
        _ => Err("Generation cancelled by user.".to_string()),
    }
}

// ============================================================================
// STATE
// ============================================================================

struct AppState {
    progress: Arc<Mutex<ProgressInfo>>,
    dialog_tx: Arc<Mutex<Option<std::sync::mpsc::Sender<String>>>>,
    /// Guard against concurrent generate calls — a second run would replace
    /// dialog_tx and leave the first run's dialogs unanswerable forever.
    busy: Arc<std::sync::atomic::AtomicBool>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            progress: Arc::new(Mutex::new(ProgressInfo::default())),
            dialog_tx: Arc::new(Mutex::new(None)),
            busy: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

#[derive(Default, Clone, Serialize)]
struct ProgressInfo {
    phase: String,
    message: String,
    done: bool,
    error: Option<String>,
    result: Option<GenerateResult>,
    question: Option<DialogQuestion>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerateResult {
    output_dir: String,
    total_files: u64,
    total_dirs: u64,
    total_size: String,
    total_output_size: String,
    chunks: usize,
    has_html: bool,
    warnings: usize,
    warning_messages: Vec<String>,
    tar_gz_path: Option<String>,
    html_path: Option<String>,
}

/// First N warning texts for display in the GUI log (mirrors the CLI cap).
fn warning_messages_capped(warnings: &[String]) -> Vec<String> {
    const CAP: usize = 100;
    let mut msgs: Vec<String> = warnings.iter().take(CAP).cloned().collect();
    if warnings.len() > CAP {
        msgs.push(format!("... and {} more", warnings.len() - CAP));
    }
    msgs
}

// ============================================================================
// DIALOG COMMANDS (using rfd directly)
// ============================================================================

#[tauri::command]
fn pick_directory(title: String) -> Option<String> {
    rfd::FileDialog::new()
        .set_title(&title)
        .pick_folder()
        .map(|p| p.to_string_lossy().to_string())
}

#[derive(Serialize)]
struct ConfigInfo {
    config: Config,
    path: Option<String>,
    error: Option<String>,
}

/// Return the effective config (branding, defaults, job-id rules) to the frontend.
#[tauri::command]
fn get_config() -> ConfigInfo {
    match Config::load(None) {
        Ok((config, path)) => ConfigInfo {
            config,
            path: path.map(|p| p.to_string_lossy().to_string()),
            error: None,
        },
        Err(e) => ConfigInfo {
            config: Config::default(),
            path: None,
            error: Some(e),
        },
    }
}

/// Persist config to the user config directory (never overwrites an
/// exe-adjacent deployment config — that one has higher priority by design).
#[tauri::command]
fn save_config(config: Config) -> Result<String, String> {
    let base = {
        #[cfg(target_os = "windows")]
        { std::env::var("APPDATA").ok().map(PathBuf::from) }
        #[cfg(not(target_os = "windows"))]
        {
            std::env::var("XDG_CONFIG_HOME").ok().map(PathBuf::from)
                .or_else(|| std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".config")))
        }
    }
    .ok_or_else(|| "Cannot determine user config directory".to_string())?;
    let dir = base.join("d2h");
    fs::create_dir_all(&dir).map_err(|e| format!("Cannot create config dir: {}", e))?;
    let path = dir.join("config.json");
    let json = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("Cannot serialize config: {}", e))?;
    fs::write(&path, json).map_err(|e| format!("Cannot write config: {}", e))?;
    Ok(path.to_string_lossy().to_string())
}

/// Pick an image file (logo) via native dialog.
#[tauri::command]
fn pick_file(title: String) -> Option<String> {
    rfd::FileDialog::new()
        .set_title(&title)
        .add_filter("Images", &["png", "svg", "jpg", "jpeg"])
        .pick_file()
        .map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
fn open_path(path: String) {
    #[cfg(target_os = "macos")]
    { let _ = std::process::Command::new("open").arg(&path).spawn(); }
    #[cfg(target_os = "windows")]
    { let _ = std::process::Command::new("explorer").arg(&path).spawn(); }
    #[cfg(target_os = "linux")]
    { let _ = std::process::Command::new("xdg-open").arg(&path).spawn(); }
}

// ============================================================================
// GENERATE COMMANDS
// ============================================================================

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GenerateArgs {
    dir: String,
    web_output: String,
    html_output: String,
    title: Option<String>,
    hidden: bool,
    include_zero_size: bool,
    include_empty_dirs: bool,
    follow_symlinks: bool,
    show_packages: bool,
    generate_web: bool,
    generate_html: bool,
    lang: String,
    theme: String,
}

#[tauri::command]
fn get_progress(state: State<'_, AppState>) -> ProgressInfo {
    state.progress.lock().unwrap().clone()
}

#[tauri::command]
fn answer_dialog(answer: String, state: State<'_, AppState>) {
    if let Some(ref tx) = *state.dialog_tx.lock().unwrap() {
        let _ = tx.send(answer);
    }
}

#[tauri::command]
async fn generate(args: GenerateArgs, state: State<'_, AppState>) -> Result<GenerateResult, String> {
    use std::sync::atomic::Ordering;

    // Reject concurrent runs (see AppState::busy).
    if state.busy.swap(true, Ordering::SeqCst) {
        return Err("Generation is already in progress.".to_string());
    }

    let progress = Arc::clone(&state.progress);

    // Create channel for frontend dialog answers
    let (tx, rx) = std::sync::mpsc::channel();
    *state.dialog_tx.lock().unwrap() = Some(tx);

    {
        let mut p = progress.lock().unwrap();
        *p = ProgressInfo {
            phase: "scanning".into(),
            message: "Starting scan...".into(),
            done: false,
            error: None,
            result: None,
            question: None,
        };
    }

    let progress_for_task = Arc::clone(&progress);
    let joined = tokio::task::spawn_blocking(move || {
        do_generate(args, progress_for_task, rx)
    })
    .await;

    state.busy.store(false, Ordering::SeqCst);
    *state.dialog_tx.lock().unwrap() = None;

    let result = joined
        .map_err(|e| format!("Task failed: {}", e))
        .and_then(|r| r);

    // Make sure the polled progress reflects failure too (not just the
    // command's return value) and never leaves a stale question behind.
    if let Err(ref e) = result {
        let mut p = progress.lock().unwrap();
        p.error = Some(e.clone());
        p.done = true;
        p.question = None;
    }

    result
}

fn do_generate(
    args: GenerateArgs,
    progress: Arc<Mutex<ProgressInfo>>,
    rx: std::sync::mpsc::Receiver<String>,
) -> Result<GenerateResult, String> {
    let dir_path = PathBuf::from(&args.dir);
    if !dir_path.is_dir() {
        let err = format!("'{}' is not a directory", args.dir);
        if let Ok(mut p) = progress.lock() {
            p.error = Some(err.clone());
            p.done = true;
        }
        return Err(err);
    }

    let root_name = args.title.clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| {
            dir_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "root".to_string())
        });

    // Output destinations come directly from GUI
    let tar_gz_dest = if args.generate_web {
        let p = PathBuf::from(&args.web_output);
        validate_output_dir(&dir_path, &p)?;
        p
    } else {
        PathBuf::new()
    };
    let html_dest = if args.generate_html {
        let p = PathBuf::from(&args.html_output);
        validate_output_dir(&dir_path, &p)?;
        p
    } else {
        PathBuf::new()
    };

    // Load site config (job-id rules, branding). Missing config = neutral defaults.
    let (config, _config_path) = Config::load(None).unwrap_or_else(|_| (Config::default(), None));

    // Use job number as filename if detected, otherwise root_name
    let source_dir_name = dir_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let case_name = extract_job_id(&source_dir_name, &config)
        .map(|(num, _)| num)
        .unwrap_or_else(|| root_name.clone());

    let tar_gz_name = case_name.clone();
    let html_name = case_name.clone();

    let case_id = tar_gz_dest
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".to_string());

    let scan_opts = ScanOptions {
        dir: dir_path,
        hidden: args.hidden,
        include_zero_size: args.include_zero_size,
        include_empty_dirs: args.include_empty_dirs,
        follow_symlinks: args.follow_symlinks,
        show_packages: args.show_packages,
    };

    let mut out_opts = output_options_from_config(&config, &args.lang, &args.theme);
    // Per-rule branding override (e.g. different logo/background for DE jobs)
    if let Some(rule) = find_matching_rule(&source_dir_name, &config) {
        if let Some(lp) = &rule.logo_path {
            out_opts.logo = Some(resolve_config_path(&config, lp));
        }
        if let Some(hb) = &rule.header_background {
            out_opts.header_background = Some(hb.clone());
        }
    }

    // Phase 1: Scan
    {
        let mut p = progress.lock().unwrap();
        p.phase = "scanning".into();
        p.message = "Scanning directory...".into();
    }

    let progress_ref = &progress;
    let scan = scan_directory(&scan_opts, Some(&|msg: &str| {
        if let Ok(mut p) = progress_ref.lock() {
            p.message = msg.to_string();
        }
    }));

    // Phase 2: Build
    {
        let mut p = progress.lock().unwrap();
        p.phase = "building".into();
        p.message = format!(
            "Building data ({} files, {} dirs)...",
            format_number(scan.total_files),
            format_number(scan.total_dirs)
        );
    }

    let subtree_stats = compute_subtree_stats(&scan.dirs);
    let (chunks, chunk_index, search_index) = build_data_structures(&scan.dirs, &subtree_stats);
    let stats_json = build_stats_json(&scan.extension_stats, scan.total_files, scan.total_size);

    // Create output directories
    if args.generate_web {
        fs::create_dir_all(&tar_gz_dest)
            .map_err(|e| format!("Failed to create web output dir: {}", e))?;
    }
    if args.generate_html {
        fs::create_dir_all(&html_dest)
            .map_err(|e| format!("Failed to create HTML output dir: {}", e))?;
    }

    // Validate web output name as job number (only when it LOOKS like one
    // per the configured warn pattern but doesn't match the valid pattern)
    let mut tar_gz_name = tar_gz_name;
    if args.generate_web && looks_like_invalid_job_id(&tar_gz_name, &config) {
        let format_hint = config
            .job_id
            .description
            .clone()
            .unwrap_or_else(|| "the configured job number format".to_string());
        if let Some(new_name) = ask_invalid_case_name(&tar_gz_name, &format_hint, &progress, &rx)? {
            tar_gz_name = new_name;
        }
    }

    // Resolve output names — ask user on collision (overwrite / rename)
    let tar_output_name = if args.generate_web {
        resolve_output_name(&tar_gz_dest, &tar_gz_name, &["tar.gz"], &progress, &rx)?
    } else {
        String::new()
    };
    let html_output_name = if args.generate_html {
        resolve_output_name(&html_dest, &html_name, &["html"], &progress, &rx)?
    } else {
        String::new()
    };

    // Phase 3: Generate web output via temp workspace (optional)
    if args.generate_web {
        {
            let mut p = progress.lock().unwrap();
            p.phase = "generating".into();
            p.message = "Generating web output...".into();
        }

        let temp_dir = create_temp_workspace()?;

        if let Err(e) = generate_multi_file_output(
            &temp_dir,
            &root_name,
            scan.total_files,
            scan.total_dirs,
            scan.total_size,
            &chunks,
            &chunk_index,
            &search_index,
            &stats_json,
            &out_opts,
            Some(&|msg: &str| {
                if let Ok(mut p) = progress_ref.lock() {
                    p.message = msg.to_string();
                }
            }),
        ) {
            remove_temp_workspace(&temp_dir);
            return Err(e);
        }

        // Write done.json to temp workspace
        let done = json!({
            "case_id": case_id,
            "generated": now_iso8601(),
            "total_files": scan.total_files,
            "total_dirs": scan.total_dirs,
            "total_size": scan.total_size,
            "total_size_human": format_size(scan.total_size),
            "has_web": args.generate_web,
            "has_html": args.generate_html,
            "warnings": scan.warnings.len(),
            "d2h_version": APP_VERSION
        });
        fs::write(
            temp_dir.join("done.json"),
            serde_json::to_string_pretty(&done).unwrap(),
        )
        .map_err(|e| format!("Failed to write done.json: {}", e))?;

        // Phase 4: tar.gz archive from temp workspace (always local first)
        {
            let mut p = progress.lock().unwrap();
            p.phase = "archiving".into();
            p.message = "Creating tar.gz archive...".into();
        }
        let local_tar = std::env::temp_dir().join(format!(
            "d2h_tar_{}_{}.tar.gz",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        ));
        if let Err(e) = create_tar_gz(&temp_dir, &local_tar) {
            remove_temp_workspace(&temp_dir);
            return Err(e);
        }

        // Move/copy tar.gz to final destination
        let final_tar = tar_gz_dest.join(format!("{}.tar.gz", tar_output_name));
        {
            let mut p = progress.lock().unwrap();
            p.message = format!("Saving tar.gz to {}...", tar_gz_dest.display());
        }
        fs::rename(&local_tar, &final_tar)
            .or_else(|_| {
                fs::copy(&local_tar, &final_tar)
                    .map(|_| ())
                    .and_then(|_| fs::remove_file(&local_tar))
            })
            .map_err(|e| format!("Failed to save tar.gz to {}: {}", final_tar.display(), e))?;

        // Remove temp workspace
        remove_temp_workspace(&temp_dir);
    }

    // Phase 5: Optional single-file HTML (generated directly to output)
    let has_html = args.generate_html;
    if args.generate_html {
        {
            let mut p = progress.lock().unwrap();
            p.phase = "html".into();
            p.message = "Generating single-file HTML...".into();
        }

        let compressed_chunks_b64 = compress_chunks(&chunks)?;
        let compressed_index_b64 = compress_and_encode(&chunk_index)?;
        let compressed_search_b64 = compress_and_encode(&search_index)?;

        let html = generate_single_file_html(
            &root_name,
            scan.total_files,
            scan.total_dirs,
            scan.total_size,
            &compressed_index_b64,
            &compressed_chunks_b64,
            &compressed_search_b64,
            &stats_json,
            &out_opts,
        )?;

        let final_html = html_dest.join(format!("{}.html", html_output_name));
        {
            let mut p = progress.lock().unwrap();
            p.message = format!("Saving HTML to {}...", html_dest.display());
        }
        fs::write(&final_html, &html)
            .map_err(|e| format!("Failed to write HTML to {}: {}", final_html.display(), e))?;
    }

    // Calculate output size from whichever output dir is available
    let primary_output = if args.generate_web { &tar_gz_dest } else { &html_dest };
    let total_output_size = dir_total_size(primary_output);

    let final_tar_gz_path = if args.generate_web {
        Some(tar_gz_dest.join(format!("{}.tar.gz", tar_output_name)).to_string_lossy().to_string())
    } else {
        None
    };
    let final_html_path = if has_html {
        Some(html_dest.join(format!("{}.html", html_output_name)).to_string_lossy().to_string())
    } else {
        None
    };

    let output_dir_str = if args.generate_web {
        args.web_output.clone()
    } else {
        args.html_output.clone()
    };

    let result = GenerateResult {
        output_dir: output_dir_str,
        total_files: scan.total_files,
        total_dirs: scan.total_dirs,
        total_size: format_size(scan.total_size),
        total_output_size: format_size(total_output_size),
        chunks: chunks.len(),
        has_html,
        warnings: scan.warnings.len(),
        warning_messages: warning_messages_capped(&scan.warnings),
        tar_gz_path: final_tar_gz_path,
        html_path: final_html_path,
    };

    {
        let mut p = progress.lock().unwrap();
        p.phase = "done".into();
        p.message = "Complete!".into();
        p.done = true;
        p.result = Some(result.clone());
    }

    Ok(result)
}

// ============================================================================
// MAIN
// ============================================================================

fn main() {
    // Catch panics and show a MessageBox instead of silent crash
    #[cfg(target_os = "windows")]
    std::panic::set_hook(Box::new(|info| {
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            format!("{}", info)
        };

        let location = info.location()
            .map(|l| format!("\r\n\r\nLocation: {}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();

        let lower = msg.to_lowercase();
        if lower.contains("webview2") || lower.contains("webview") {
            show_error_dialog(
                "D2H \u{2014} WebView2 Required",
                "Microsoft WebView2 Runtime is not installed.\r\n\r\n\
                 D2H requires WebView2 to run.\r\n\r\n\
                 Please install it from:\r\n\
                 https://developer.microsoft.com/microsoft-edge/webview2/\r\n\
                 (or contact your administrator)\r\n\r\n\
                 After installation, restart D2H.",
            );
        } else {
            show_error_dialog(
                "D2H \u{2014} Error",
                &format!("D2H crashed:\r\n\r\n{}{}", msg, location),
            );
        }
    }));

    let result = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            pick_directory,
            open_path,
            generate,
            get_progress,
            answer_dialog,
            get_config,
            save_config,
            pick_file
        ])
        .run(tauri::generate_context!());

    if let Err(e) = result {
        let err_msg = e.to_string();

        #[cfg(target_os = "windows")]
        {
            let err_lower = err_msg.to_lowercase();
            if err_lower.contains("webview2") || err_lower.contains("webview") {
                show_error_dialog(
                    "D2H \u{2014} WebView2 Required",
                    "Microsoft WebView2 Runtime is not installed.\r\n\r\n\
                     D2H requires WebView2 to run.\r\n\r\n\
                     Please install it from:\r\n\
                     https://developer.microsoft.com/microsoft-edge/webview2/\r\n\r\n\
                     After installation, restart D2H.",
                );
            } else {
                show_error_dialog(
                    "D2H \u{2014} Error",
                    &format!("D2H failed to start:\r\n\r\n{}", err_msg),
                );
            }
        }

        std::process::exit(1);
    }
}
