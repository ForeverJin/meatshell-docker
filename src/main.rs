//! Entry point for the headless MeatShell build.
//!
//! There is no desktop UI in this tree. The executable serves two purposes:
//! the scriptable `cli` automation commands and the authenticated MCP HTTP
//! service, both operating on a selected profile directory.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod allocator;

#[global_allocator]
static GLOBAL: allocator::Allocator = allocator::Allocator;

// The SSH/SFTP runtime is inherited from the desktop tree. A lot of its API
// (terminal events, GUI-only session plumbing, tunnel helpers) is no longer
// called now that the UI is gone, but it is kept for upstream merges and for
// the automation paths that still use the same modules. Silence dead-code
// noise per inherited module instead of hiding it crate-wide; the new
// headless-only code (main, cli, mcp, logging) keeps the lint.
#[allow(dead_code)]
mod automation;
mod cli;
#[allow(dead_code)]
mod config;
#[allow(dead_code)]
mod i18n;
mod logging;
mod mcp;
#[allow(dead_code)]
mod sftp;
#[allow(dead_code)]
mod ssh;
#[allow(dead_code)]
mod terminal;
#[allow(dead_code)]
mod tunnel;

enum StartMode {
    Mcp,
    Cli,
    Usage,
}

impl StartMode {
    fn detect(args: &[String]) -> Self {
        match args.get(1).map(String::as_str) {
            Some("mcp") if args.get(2).is_some_and(|arg| arg == "serve") => Self::Mcp,
            Some("cli") => Self::Cli,
            _ => Self::Usage,
        }
    }
}

fn main() -> anyhow::Result<()> {
    let mut args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--http-config") {
        anyhow::ensure!(args.iter().any(|arg| arg == "--data-dir") || std::env::var_os("MEATSHELL_DATA_DIR").is_some(),
            "HTTP service requires an explicitly selected --data-dir or MEATSHELL_DATA_DIR profile");
    }
    config::configure_profile(&mut args)?;
    if args.iter().any(|arg| arg == "--config-info") {
        let store = config::ConfigStore::load()?;
        println!(
            "{}",
            serde_json::json!({
                "executable": std::env::current_exe()?,
                "version": env!("CARGO_PKG_VERSION"),
                "data_dir": config::data_dir(),
                "session_count": store.sessions().len(),
            })
        );
        return Ok(());
    }

    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("meatshell {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if let Ok(code) = std::env::var("MEATSHELL_LANG") {
        i18n::set_language(&code);
    }

    let mode = StartMode::detect(&args);
    init_tracing();

    match mode {
        StartMode::Mcp => mcp::run(&args),
        StartMode::Cli => cli::run(&args),
        StartMode::Usage => anyhow::bail!(
            "usage: meatshell cli <command> | meatshell mcp serve [--http-config <file>]\n\
             try `meatshell cli help`"
        ),
    }
}

/// Set up tracing: stderr (honours RUST_LOG, default info) **plus** a capped
/// `error.log` file at WARN and above so operators can collect diagnostics.
fn init_tracing() {
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::{fmt, EnvFilter};

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let stderr_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(env_filter);

    // One file, capped at 50 MiB, auto-overwriting when full.
    let file_layer = logging::path()
        .and_then(|p| logging::CappedFile::open(p, 50 * 1024 * 1024).ok())
        .map(|cf| {
            fmt::layer()
                .with_ansi(false)
                .with_writer(logging::CappedWriter::new(cf))
                .with_filter(EnvFilter::new("warn"))
        });

    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layer)
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_start_mode() {
        let mcp = vec!["meatshell".to_string(), "mcp".to_string(), "serve".to_string()];
        assert!(matches!(StartMode::detect(&mcp), StartMode::Mcp));

        let cli = vec![
            "meatshell".to_string(),
            "cli".to_string(),
            "sessions".to_string(),
        ];
        assert!(matches!(StartMode::detect(&cli), StartMode::Cli));

        let bare = vec!["meatshell".to_string()];
        assert!(matches!(StartMode::detect(&bare), StartMode::Usage));
    }
}
