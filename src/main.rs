use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, MouseButton, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

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
            let mut stderr = io::stderr();
            let _ = execute!(stderr, DisableMouseCapture, LeaveAlternateScreen);
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
    // (clicked path, time) of the last mouse click, for double-click detection.
    let mut last_click: Option<(PathBuf, Instant)> = None;
    const DOUBLE_CLICK_MS: u128 = 400;

    loop {
        terminal
            .draw(|f| {
                let ui = TreeWidget::new(&mut app);
                f.render_widget(ui, f.area());
            })
            .map_err(io::Error::other)?;

        if !event::poll(Duration::from_millis(250))? {
            continue;
        }

        match event::read()? {
            Event::Key(key) if key.kind == event::KeyEventKind::Press => {
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
                } else if app.search_mode {
                    match key.code {
                        KeyCode::Esc => app.exit_search(),
                        KeyCode::Backspace | KeyCode::Delete => app.search_backspace(),
                        KeyCode::Tab => app.mode.toggle(),
                        KeyCode::Up => app.move_selection(-1),
                        KeyCode::Down => app.move_selection(1),
                        KeyCode::Right => app.expand_current(),
                        KeyCode::Left => app.on_left(),
                        KeyCode::Enter => {
                            if let Some(result) = app.record_and_get_path() {
                                return Ok(Some(result));
                            }
                        }
                        KeyCode::Char(c) if !c.is_control() => app.search_input(c),
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Char(' ') => {
                            app.toggle_history_mode();
                        }
                        KeyCode::Char('q') | KeyCode::Esc => return Ok(None),
                        KeyCode::Tab => app.mode.toggle(),
                        KeyCode::Char('f') => app.start_search(),
                        KeyCode::Char('v') => app.toggle_show_files(),
                        KeyCode::Char('a') => app.toggle_show_hidden(),
                        KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
                        KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
                        KeyCode::Right | KeyCode::Char('l') => app.expand_current(),
                        KeyCode::Left | KeyCode::Char('h') => app.on_left(),
                        KeyCode::Char('t') => {
                            let now = Instant::now();
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
            Event::Mouse(mouse) => {
                let area: Rect = terminal.size().map_err(io::Error::other)?.into();
                let list_area = list_content_area(area);

                // Ignore clicks outside the list region.
                let in_list =
                    mouse.row >= list_area.top() && mouse.row < list_area.top() + list_area.height;

                match mouse.kind {
                    MouseEventKind::ScrollUp if in_list => {
                        if app.history_mode {
                            app.scroll_history(-1);
                        } else {
                            app.scroll(-1);
                        }
                    }
                    MouseEventKind::ScrollDown if in_list => {
                        if app.history_mode {
                            app.scroll_history(1);
                        } else {
                            app.scroll(1);
                        }
                    }
                    MouseEventKind::Down(MouseButton::Left) if in_list => {
                        let offset = if app.history_mode {
                            app.history_list_state.offset()
                        } else {
                            app.list_state.offset()
                        };
                        let idx = (mouse.row - list_area.top()) as usize + offset;

                        if app.history_mode {
                            if idx >= app.history.entries.len() {
                                last_click = None;
                                continue;
                            }
                            app.history_list_state.select(Some(idx));
                            let clicked_path = app.history.entries[idx].path.clone();
                            let now = Instant::now();
                            let is_double = last_click
                                .as_ref()
                                .map(|(p, t)| {
                                    *p == clicked_path
                                        && now.duration_since(*t).as_millis() < DOUBLE_CLICK_MS
                                })
                                .unwrap_or(false);
                            if is_double {
                                if let Some(result) = app.select_from_history() {
                                    return Ok(Some(result));
                                }
                                last_click = None;
                            } else {
                                last_click = Some((clicked_path, now));
                            }
                        } else {
                            let visible = app.get_visible_nodes();
                            if idx >= visible.len() {
                                last_click = None;
                                continue;
                            }
                            let clicked_path = visible[idx].1.path.clone();
                            app.select_visible_index(idx);

                            let now = Instant::now();
                            let is_double = last_click
                                .as_ref()
                                .map(|(p, t)| {
                                    *p == clicked_path
                                        && now.duration_since(*t).as_millis() < DOUBLE_CLICK_MS
                                })
                                .unwrap_or(false);

                            if is_double {
                                if let Some(result) = app.record_and_get_path() {
                                    return Ok(Some(result));
                                }
                                last_click = None;
                            } else {
                                // Single click toggles expansion of the clicked directory.
                                app.toggle_current();
                                last_click = Some((clicked_path, now));
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

/// Compute the inner list area that the tree/history `List` widget is rendered into,
/// mirroring the layout in `ui.rs` (outer double-bordered block + vertical [Min(1), Length(2)]).
fn list_content_area(area: Rect) -> Rect {
    let content = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    Rect {
        x: content.x,
        y: content.y,
        width: content.width,
        height: content.height.saturating_sub(2),
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

    #[test]
    fn list_content_area_matches_ui_layout() {
        // Outer block has a 1-cell border on every side; the vertical layout
        // reserves 2 rows for the guide, leaving the rest for the list.
        let area = Rect::new(0, 0, 80, 24);
        let list = list_content_area(area);
        assert_eq!(list, Rect::new(1, 1, 78, 20));
    }

    #[test]
    fn list_content_area_handles_small_terminal() {
        let area = Rect::new(0, 0, 40, 5);
        let list = list_content_area(area);
        // 5 - 2 (borders) - 2 (guide) = 1 row for the list
        assert_eq!(list.height, 1);
    }
}
