use std::io;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use anyhow::Result;
use clap::Parser;

mod app;
mod config;
mod ui;
mod shell;
#[cfg(test)]
mod test_support;

use app::App;
use ui::TreeWidget;
use shell::{has_shell_integration, setup_shell_integration};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
#[command(disable_version_flag = true)]
struct Cli {
    /// Setup shell integration
    #[arg(short, long)]
    setup: bool,

    /// Print version information
    #[arg(short = 'v', long = "version")]
    version: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.version {
        eprintln!("cdtree {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if cli.setup {
        setup_shell_integration()?;
        return Ok(());
    }

    if !has_shell_integration()? {
        print_install_required();
        return Ok(());
    }

    // Setup terminal
    enable_raw_mode()?;
    // Use stderr for TUI to leave stdout clean for piping the result
    let mut stderr = io::stderr();
    execute!(stderr, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stderr);
    let mut terminal = Terminal::new(backend)?;

    // Create app
    let app = App::new()?;
    let res = run_app(&mut terminal, app);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    match res {
        Ok(Some(path)) => println!("{}", path),
        Err(err) => eprintln!("{:?}", err),
        _ => {}
    }

    Ok(())
}

fn print_install_required() {
    eprintln!("cdtree shell integration is not set up, so it cannot start.");
    eprintln!("Please run the following:\n");
    eprintln!("  cdtree --setup");
    eprintln!("\nAfter setting it up, reload your shell.");
}

fn run_app<B: ratatui::backend::Backend>(terminal: &mut Terminal<B>, mut app: App) -> io::Result<Option<String>> 
where
    <B as ratatui::backend::Backend>::Error: std::error::Error + Send + Sync + 'static,
{
    loop {
        terminal.draw(|f| {
            let ui = TreeWidget::new(&mut app);
            f.render_widget(ui, f.area());
        }).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        if event::poll(std::time::Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(None),
                    KeyCode::Char('f') => app.toggle_show_files(),
                    KeyCode::Char('a') => app.toggle_show_hidden(),
                    KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
                    KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
                    KeyCode::Right | KeyCode::Char('l') => app.expand_current(),
                    KeyCode::Left | KeyCode::Char('h') => app.on_left(),
                    KeyCode::Char('t') => {
                        let now = std::time::Instant::now();
                        if let Some(last) = app.last_theme_change {
                            if now.duration_since(last).as_millis() < 200 {
                                 app.reset_theme_default();
                                 app.last_theme_change = Some(now);
                                 continue;
                            }
                        }
                        app.change_theme_random();
                        app.last_theme_change = Some(now);
                    }
                    KeyCode::Enter => {
                        if app.is_selected_dir() {
                            let path = app.selected_path.to_string_lossy().to_string();
                            return Ok(Some(path));
                        }
                    }
                     _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn verify_cli() {
        Cli::command().debug_assert();
    }

    #[test]
    fn verify_setup_arg() {
        let cli = Cli::try_parse_from(&["cdtree", "--setup"]).unwrap();
        assert!(cli.setup);

        let cli = Cli::try_parse_from(&["cdtree", "-s"]).unwrap();
        assert!(cli.setup);
    }
}
