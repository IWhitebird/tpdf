mod app;
mod cache;
mod input;
mod pdf;
mod update;
mod view;

use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

use clap::{Parser, Subcommand};
use ratatui_image::picker::Picker;

use app::{AppConfig, PageLayout};

#[derive(Parser)]
#[command(name = "tpdf", about = "Terminal PDF viewer", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Path to PDF file
    path: Option<String>,

    /// Start in night mode
    #[arg(short, long)]
    night: bool,

    /// Start in fullscreen
    #[arg(short, long)]
    fullscreen: bool,

    /// Start at page number
    #[arg(short, long, value_name = "N")]
    page: Option<usize>,

    /// Layout: 1 (single), 2 (dual), 3 (triple)
    #[arg(short = 'd', long, value_name = "1|2|3")]
    layout: Option<u8>,

    /// Start in text-only mode (works on any terminal)
    #[arg(short, long)]
    text: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Update tpdf to the latest version
    Update,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    if matches!(cli.command, Some(Command::Update)) {
        return update::self_update();
    }

    let Some(path) = cli.path else {
        eprintln!("tpdf - Terminal PDF viewer\n");
        eprintln!("Usage: tpdf <file.pdf>");
        eprintln!("       tpdf update\n");
        eprintln!("Run 'tpdf --help' for more options.");
        std::process::exit(1);
    };

    let (picker, text_mode) = if cli.text {
        (None, true)
    } else if let Ok(p) = Picker::from_query_stdio() {
        (Some(p), false)
    } else {
        eprintln!("Note: terminal does not support image protocols, using text mode.");
        (None, true)
    };

    let config = AppConfig {
        dark_mode: cli.night,
        fullscreen: cli.fullscreen,
        start_page: cli.page.unwrap_or(1).saturating_sub(1),
        layout: match cli.layout {
            Some(2) => PageLayout::Dual,
            Some(3) => PageLayout::Triple,
            _ => PageLayout::Single,
        },
        text_mode,
    };

    let (term_cols, term_rows) = crossterm::terminal::size()?;

    let mut app = app::App::new(&path, picker, term_cols, term_rows, &config)?;

    let mut terminal = ratatui::init();
    let result = app.run(&mut terminal);
    ratatui::restore();

    result?;
    Ok(())
}
