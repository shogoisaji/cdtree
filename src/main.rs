use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;
use std::path::Path;
use std::process::Command;

// Command constants
#[cfg(target_os = "macos")]
const OPEN_COMMAND: &str = "open";
#[cfg(target_os = "linux")]
const OPEN_COMMAND: &str = "xdg-open";
#[cfg(target_os = "windows")]
const OPEN_COMMAND: &str = "explorer";

const CODE_COMMAND: &str = "code";

fn execute_mode_action(path: &str, mode: app::AppMode) {
    match mode {
        app::AppMode::Cd => {
            if is_safe_cd_target(path) {
                println!("{}", path);
            } else {
                eprintln!("Refusing unsafe cd target: {:?}", path);
            }
        }
        app::AppMode::Open => {
            if let Err(e) = Command::new(OPEN_COMMAND).arg(path).spawn() {
                eprintln!("Failed to open: {}", e);
            } else {
                eprintln!("Opened: {:?}", path);
            }
        }
        app::AppMode::Code => {
            if let Err(e) = Command::new(CODE_COMMAND).arg(path).spawn() {
                eprintln!("Failed to open in VS Code: {}", e);
            } else {
                eprintln!("Opened in VS Code: {:?}", path);
            }
        }
    }
}

fn is_safe_cd_target(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\0')
        && !path.contains('\n')
        && !path.contains('\r')
        && Path::new(path).is_absolute()
        && Path::new(path).is_dir()
}

mod app;
mod config;
mod history;
mod shell;
#[cfg(test)]
mod test_support;
mod ui;

use app::App;
use shell::{
    has_shell_integration, print_shell_integration_status, setup_shell_integration,
    uninstall_shell_integration,
};
use ui::TreeWidget;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
#[command(disable_version_flag = true)]
struct Cli {
    /// Setup shell integration
    #[arg(short, long)]
    setup: bool,

    /// Remove shell integration from your shell rc file
    #[arg(long)]
    uninstall: bool,

    /// Print shell integration diagnostics
    #[arg(long)]
    doctor: bool,

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

    if cli.uninstall {
        uninstall_shell_integration()?;
        return Ok(());
    }

    if cli.doctor {
        print_shell_integration_status()?;
        return Ok(());
    }

    if !has_shell_integration()? {
        print_install_required();
        return Ok(());
    }

    let app = App::new()?;

    // Setup terminal
    enable_raw_mode()?;
    // Use stderr for TUI to leave stdout clean for piping the result
    let mut stderr = io::stderr();
    if let Err(err) = execute!(stderr, EnterAlternateScreen, EnableMouseCapture) {
        let _ = disable_raw_mode();
        return Err(err.into());
    }
    let backend = CrosstermBackend::new(stderr);
    let mut terminal = match Terminal::new(backend) {
        Ok(terminal) => terminal,
        Err(err) => {
            let _ = disable_raw_mode();
            return Err(err.into());
        }
    };

    let res = run_app(&mut terminal, app);

    // Restore terminal
    restore_terminal(&mut terminal)?;

    match res {
        Ok(Some((path, mode))) => {
            execute_mode_action(&path, mode);
        }
        Err(err) => eprintln!("{:?}", err),
        _ => {}
    }

    Ok(())
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stderr>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn print_install_required() {
    eprintln!("cdtree shell integration is not set up, so it cannot start.");
    eprintln!("Please run the following:\n");
    eprintln!("  cdtree --setup");
    eprintln!("\nAfter setting it up, reload your shell.");
}

fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    mut app: App,
) -> io::Result<Option<(String, app::AppMode)>>
where
    <B as ratatui::backend::Backend>::Error: std::error::Error + Send + Sync + 'static,
{
    loop {
        terminal
            .draw(|f| {
                let ui = TreeWidget::new(&mut app);
                f.render_widget(ui, f.area());
            })
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

        if event::poll(std::time::Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != event::KeyEventKind::Press {
                    continue;
                }
                if app.history_mode {
                    match key.code {
                        KeyCode::Char(' ') | KeyCode::Esc => {
                            app.toggle_history_mode();
                        }
                        KeyCode::Char('q') => return Ok(None),
                        KeyCode::Tab => app.mode.toggle(),
                        KeyCode::Up | KeyCode::Char('k') => {
                            app.move_history_selection(-1);
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            app.move_history_selection(1);
                        }
                        KeyCode::Enter => {
                            if let Some(result) = app.select_from_history() {
                                return Ok(Some(result));
                            }
                        }
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Char(' ') => {
                            app.toggle_history_mode();
                        }
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(None),
                        KeyCode::Tab => app.mode.toggle(),
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
                            if let Some(result) = app.record_and_get_path() {
                                return Ok(Some(result));
                            }
                        }
                        _ => {}
                    }
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

    #[test]
    fn verify_uninstall_arg() {
        let cli = Cli::try_parse_from(&["cdtree", "--uninstall"]).unwrap();
        assert!(cli.uninstall);
    }

    #[test]
    fn verify_doctor_arg() {
        let cli = Cli::try_parse_from(&["cdtree", "--doctor"]).unwrap();
        assert!(cli.doctor);
    }

    #[test]
    fn cd_target_rejects_control_chars_and_relative_paths() {
        assert!(!is_safe_cd_target(""));
        assert!(!is_safe_cd_target("relative/path"));
        assert!(!is_safe_cd_target("/tmp/a\nb"));
        assert!(!is_safe_cd_target("/tmp/a\rb"));
        assert!(!is_safe_cd_target("/tmp/a\0b"));
    }

    #[test]
    fn cd_target_accepts_existing_absolute_directory() {
        assert!(is_safe_cd_target(std::env::temp_dir().to_str().unwrap()));
    }
}
