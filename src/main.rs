mod app;
mod encoder;
mod model;
mod preset;
mod prober;
mod scanner;
mod ui;

use std::io;
use std::path::PathBuf;

use clap::Parser;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

#[derive(Parser)]
#[command(
    name = "mcc",
    about = "Media Control Center - inspect your media library"
)]
struct Cli {
    /// Root directory to scan for media files
    path: PathBuf,

    /// Path to encoding.yaml config file
    #[arg(short, long)]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let cli = Cli::parse();

    if !cli.path.is_dir() {
        eprintln!("Error: {:?} is not a valid directory", cli.path);
        std::process::exit(1);
    }

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Load encoding presets: explicit --config path, or auto-discover in CWD
    let config = match &cli.config {
        Some(path) => match preset::load_presets_from(path) {
            Ok(c) => c,
            Err(e) => {
                disable_raw_mode()?;
                execute!(
                    terminal.backend_mut(),
                    LeaveAlternateScreen,
                    DisableMouseCapture
                )?;
                eprintln!("Error loading config {:?}: {}", path, e);
                std::process::exit(1);
            }
        },
        None => preset::load_presets(&std::env::current_dir().unwrap_or_default()),
    };

    // Run app - scan streams in the background
    let root_path = cli.path.canonicalize().unwrap_or(cli.path);
    let mut app = app::App::new(root_path, config);

    // Clean stale temp files from previous runs / crashes
    app.cleanup_temp_dirs();

    let result = run_app(&mut terminal, &mut app);

    // Kill any running encode and clean temp files before exit
    if app.is_encoding_active() {
        app.cancel_all();
    }
    app.cleanup_temp_dirs();

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }

    Ok(())
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut app::App,
) -> io::Result<()> {
    loop {
        app.poll_scan_results();
        app.poll_probe_results();
        app.poll_encode_events();

        terminal.draw(|f| ui::draw(f, app))?;

        app.handle_event()?;

        if app.should_quit {
            return Ok(());
        }
    }
}
