mod app;
mod cache;
mod patterns;
mod scanner;
mod ui;

use anyhow::Result;
use app::App;
use clap::Parser;
use crossterm::event::{self, Event};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "space-check", about = "TUI disk space analyzer")]
struct Cli {
    /// Directory to scan (defaults to current directory)
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Large file threshold in MB
    #[arg(short, long, default_value_t = 100)]
    threshold: u64,

    /// Enable disk caching of scan results (5 min TTL)
    #[arg(long)]
    cache: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let root = cli.path.canonicalize()?;
    let threshold_bytes = cli.threshold * 1024 * 1024;

    // Set up terminal
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    // Run app (handles its own scanning, with optional cache)
    let mut app = App::new(root, threshold_bytes, cli.cache);
    let result = run_event_loop(&mut terminal, &mut app);

    // Cleanup terminal — always runs
    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;

    result
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        // Drain scanner and delete messages
        app.poll_scanner();
        app.poll_delete();

        // Render
        terminal.draw(|f| ui::draw(f, app))?;

        if app.should_quit {
            break;
        }

        // Poll input — shorter timeout while scanning/deleting for responsive progress updates
        let timeout = if app.scanning || app.is_deleting() {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(250)
        };

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                app.handle_key(key);
            }
        }
    }
    Ok(())
}
