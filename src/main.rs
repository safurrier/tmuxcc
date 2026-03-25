use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

use tmuxcc::app::Config;
use tmuxcc::logging;
use tmuxcc::ui::run_app;

#[derive(Parser)]
#[command(name = "tmuxcc")]
#[command(author, version, about, long_about = None)]
#[command(
    about = "AI Agent Dashboard for tmux - Claude Code, OpenCode, Codex CLI, Gemini CLI を一元管理"
)]
struct Cli {
    /// ポーリング間隔（ミリ秒）
    #[arg(short, long, default_value = "500", value_name = "MS")]
    poll_interval: u64,

    /// ペインからキャプチャする行数
    #[arg(short, long, default_value = "100", value_name = "LINES")]
    capture_lines: u32,

    /// 設定ファイルのパス
    #[arg(short = 'f', long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Enable verbose debug logging (logs always written to ~/.local/state/tmuxcc/)
    #[arg(short, long)]
    debug: bool,

    /// Running inside a tmux popup (auto-quit on focus/go)
    #[arg(long)]
    popup: bool,

    /// Disable desktop notifications
    #[arg(long)]
    no_notifications: bool,

    /// 設定ファイルのパスを表示
    #[arg(long)]
    show_config_path: bool,

    /// デフォルト設定ファイルを生成
    #[arg(long)]
    init_config: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Show config path and exit
    if cli.show_config_path {
        if let Some(path) = Config::default_path() {
            println!("{}", path.display());
        } else {
            println!("設定ディレクトリが見つかりません");
        }
        return Ok(());
    }

    // Initialize config file and exit
    if cli.init_config {
        let config = Config::default();
        if let Err(e) = config.save() {
            eprintln!("設定ファイルの作成に失敗: {}", e);
            std::process::exit(1);
        }
        if let Some(path) = Config::default_path() {
            println!("設定ファイルを作成しました: {}", path.display());
        }
        return Ok(());
    }

    // Setup logging (always on, --debug for verbose)
    logging::init(cli.debug);

    // Load config (from file or CLI args)
    let mut config = if let Some(config_path) = &cli.config {
        Config::load_from(config_path).unwrap_or_else(|e| {
            eprintln!("設定ファイルの読み込みに失敗: {}", e);
            std::process::exit(1);
        })
    } else {
        Config::load()
    };

    // CLI args override config file
    config.poll_interval_ms = cli.poll_interval;
    config.capture_lines = cli.capture_lines;
    config.popup = cli.popup;
    if cli.no_notifications {
        config.notifications.enabled = false;
    }

    // Run the application
    run_app(config).await
}
