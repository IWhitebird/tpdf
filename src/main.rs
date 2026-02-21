mod app;
mod cache;
mod dark;
mod input;
mod pdf;
mod view;

use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

use clap::Parser;
use ratatui_image::picker::Picker;

#[derive(Parser)]
#[command(name = "tpdf", about = "Terminal PDF viewer")]
struct Cli {
    /// Path to PDF file
    path: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Must query stdio before entering alternate screen
    let picker = Picker::from_query_stdio()?;
    let (term_cols, term_rows) = crossterm::terminal::size()?;

    let mut app = app::App::new(&cli.path, picker, term_cols, term_rows)?;

    let mut terminal = ratatui::init();
    let result = app.run(&mut terminal);
    ratatui::restore();

    result?;
    Ok(())
}
