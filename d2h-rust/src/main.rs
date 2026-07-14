//! D2H CLI — command-line interface for directory scanning.

use clap::Parser;
use d2h::*;
use indicatif::{ProgressBar, ProgressStyle};
use serde_json::json;
use std::fs;
use std::path::PathBuf;

/// D2H — Directory to HTML generator
#[derive(Parser, Debug)]
#[command(name = APP_NAME, version = APP_VERSION, about = "Generates browseable HTML from directory structure")]
struct Args {
    /// Directory to scan
    #[arg(short, long)]
    dir: PathBuf,

    /// Output directory (for web output, default mode)
    #[arg(short, long)]
    output: PathBuf,

    /// Also generate single-file HTML (embedded data, for email attachment)
    #[arg(long, default_value_t = false)]
    html: bool,


    /// Custom title (default: directory name)
    #[arg(short, long)]
    title: Option<String>,

    /// Include hidden files and directories
    #[arg(long, default_value_t = false)]
    hidden: bool,

    /// Include zero-size files (default: excluded)
    #[arg(long, default_value_t = false)]
    include_zero_size: bool,

    /// Include empty directories (default: pruned)
    #[arg(long, default_value_t = false)]
    include_empty_dirs: bool,

    /// Follow symbolic links (default: don't follow)
    #[arg(long, default_value_t = false)]
    follow_symlinks: bool,

    /// Show macOS packages (.app etc.) as directories instead of files
    #[arg(long, default_value_t = false)]
    show_packages: bool,

    /// Override embedded logo with custom SVG
    #[arg(long)]
    logo: Option<PathBuf>,

    /// Override embedded pako.min.js with custom file
    #[arg(long)]
    pako: Option<PathBuf>,

    /// Output language: en, cz, de, it, es, fr, pl (default: en)
    #[arg(long, default_value = "en")]
    lang: String,

    /// Visual theme: default, nologo (default: default)
    #[arg(long, default_value = "default")]
    theme: String,

    /// Path to d2h.config.json (default: auto-discover next to the executable
    /// or in the user config directory)
    #[arg(long)]
    config: Option<PathBuf>,
}

/// Print an error message and exit — used for fatal I/O failures.
fn fail(msg: &str) -> ! {
    eprintln!("Error: {}", msg);
    std::process::exit(1);
}

fn main() {
    let args = Args::parse();

    if !args.dir.is_dir() {
        eprintln!("Error: '{}' is not a directory", args.dir.display());
        std::process::exit(1);
    }

    // Validate output directory safety
    if let Err(e) = validate_output_dir(&args.dir, &args.output) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }

    let root_name = args.title.clone().unwrap_or_else(|| {
        args.dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "root".to_string())
    });

    let case_id = args
        .output
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".to_string());

    let scan_opts = ScanOptions {
        dir: args.dir.clone(),
        hidden: args.hidden,
        include_zero_size: args.include_zero_size,
        include_empty_dirs: args.include_empty_dirs,
        follow_symlinks: args.follow_symlinks,
        show_packages: args.show_packages,
    };

    let (config, config_path) = Config::load(args.config.as_deref()).unwrap_or_else(|e| fail(&e));

    let mut out_opts = output_options_from_config(&config, &args.lang, &args.theme);
    // Explicit CLI flags override config
    if args.logo.is_some() {
        out_opts.logo = args.logo.clone();
    }
    out_opts.pako = args.pako.clone();

    println!("{} v{}", APP_NAME, APP_VERSION);
    if let Some(cp) = &config_path {
        println!("Config: {}", cp.display());
    }
    println!("Scanning: {}", args.dir.display());

    // Phase 1: Scan with CLI progress
    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")
            .unwrap(),
    );

    let scan = scan_directory(&scan_opts, Some(&|msg: &str| {
        pb.set_message(msg.to_string());
        pb.tick();
    }));

    pb.finish_and_clear();
    println!(
        "Found {} files in {} folders ({})",
        format_number(scan.total_files),
        format_number(scan.total_dirs),
        format_size(scan.total_size)
    );

    if !scan.warnings.is_empty() {
        println!("\n--- {} warnings during scan ---", scan.warnings.len());
        for (i, warning) in scan.warnings.iter().enumerate() {
            if i < 50 {
                println!("  {}", warning);
            } else {
                println!("  ... and {} more", scan.warnings.len() - 50);
                break;
            }
        }
        println!("---\n");
    }

    // Phase 2: Build data structures
    let subtree_stats = compute_subtree_stats(&scan.dirs);
    let (chunks, chunk_index, search_index) = build_data_structures(&scan.dirs, &subtree_stats);
    let stats_json = build_stats_json(&scan.extension_stats, scan.total_files, scan.total_size);

    // Create output directory
    fs::create_dir_all(&args.output).expect("Failed to create output directory");

    // Find unique name for output files (avoids overwriting existing files)
    let extensions: Vec<&str> = if args.html {
        vec!["tar.gz", "html"]
    } else {
        vec!["tar.gz"]
    };
    let output_name = unique_name(&args.output, &root_name, &extensions);
    if output_name != root_name {
        println!("Note: Using '{}' to avoid overwriting existing files.", output_name);
    }

    // Phase 3: Generate web output to temp workspace
    println!("Generating web output...");
    let temp_dir = create_temp_workspace().unwrap_or_else(|e| fail(&e));

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
        None,
    ) {
        remove_temp_workspace(&temp_dir);
        fail(&e);
    }

    // Write done.json to temp workspace
    let done = json!({
        "case_id": case_id,
        "generated": now_iso8601(),
        "total_files": scan.total_files,
        "total_dirs": scan.total_dirs,
        "total_size": scan.total_size,
        "total_size_human": format_size(scan.total_size),
        "has_web": true,
        "has_html": args.html,
        "warnings": scan.warnings.len(),
        "d2h_version": APP_VERSION
    });
    if let Err(e) = fs::write(
        temp_dir.join("done.json"),
        serde_json::to_string_pretty(&done).unwrap(),
    ) {
        remove_temp_workspace(&temp_dir);
        fail(&format!("Failed to write done.json: {}", e));
    }

    // Phase 4: Create tar.gz archive from temp workspace
    println!("Creating tar.gz archive...");
    let tar_path = args.output.join(format!("{}.tar.gz", output_name));
    if let Err(e) = create_tar_gz(&temp_dir, &tar_path) {
        remove_temp_workspace(&temp_dir);
        fail(&e);
    }
    let tar_size = fs::metadata(&tar_path).map(|m| m.len()).unwrap_or(0);
    println!("  Archive: {} ({})", tar_path.display(), format_size(tar_size));

    // Remove temp workspace
    remove_temp_workspace(&temp_dir);
    println!("  Web: {} chunks", chunks.len());

    // Phase 5: Optional single-file HTML (generated directly to output)
    if args.html {
        println!("Generating single-file HTML...");
        let compressed_chunks_b64 = compress_chunks(&chunks).unwrap_or_else(|e| fail(&e));
        let compressed_index_b64 = compress_and_encode(&chunk_index).unwrap_or_else(|e| fail(&e));
        let compressed_search_b64 = compress_and_encode(&search_index).unwrap_or_else(|e| fail(&e));

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
        )
        .unwrap_or_else(|e| fail(&e));

        let html_path = args.output.join(format!("{}.html", output_name));
        fs::write(&html_path, &html)
            .unwrap_or_else(|e| fail(&format!("Failed to write HTML file: {}", e)));
        let html_size = fs::metadata(&html_path).map(|m| m.len()).unwrap_or(0);
        println!("  HTML: {} ({})", html_path.display(), format_size(html_size));
    }

    let total_output_size = dir_total_size(&args.output);
    println!(
        "\nOutput: {} ({})",
        args.output.display(),
        format_size(total_output_size)
    );
    println!("Done!");
}
