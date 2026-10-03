mod app;
mod editor;
mod init;
mod integration;
mod link;
mod starlight;
mod state;
mod view;

use std::io::{self, IsTerminal};
use std::panic::{self, PanicHookInfo};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use akasha_core::ResolveRequest;
use crossterm::{
    cursor::Show,
    event::{
        self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
        EnableFocusChange, EnableMouseCapture, Event, KeyEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use app::App;

const AUTO_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

type PanicHook = dyn Fn(&PanicHookInfo<'_>) + Send + Sync + 'static;

/// Restore before the panic hook prints, so its diagnostic survives the alternate screen.
struct TerminalGuard {
    restored: Arc<AtomicBool>,
    previous_hook: Arc<PanicHook>,
}

impl TerminalGuard {
    fn new() -> Self {
        let restored = Arc::new(AtomicBool::new(false));
        let previous_hook: Arc<PanicHook> = Arc::from(panic::take_hook());
        let hook_restored = Arc::clone(&restored);
        let hook_previous = Arc::clone(&previous_hook);
        panic::set_hook(Box::new(move |info| {
            restore_terminal_once(&hook_restored);
            hook_previous(info);
        }));
        Self {
            restored,
            previous_hook,
        }
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal_once(&self.restored);
        if !std::thread::panicking() {
            let _ = panic::take_hook();
            let previous_hook = Arc::clone(&self.previous_hook);
            panic::set_hook(Box::new(move |info| previous_hook(info)));
        }
    }
}

fn restore_terminal_once(restored: &AtomicBool) {
    if !restored.swap(true, Ordering::AcqRel) {
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            DisableFocusChange,
            DisableMouseCapture,
            Show,
            LeaveAlternateScreen
        );
        let _ = disable_raw_mode();
    }
}

pub(crate) fn run(
    root: Option<PathBuf>,
    project: Option<String>,
    json: bool,
    no_color: bool,
    no_motion: bool,
    ascii: bool,
) -> Result<(), u8> {
    if json
        || !io::stdin().is_terminal()
        || !io::stdout().is_terminal()
        || std::env::var("TERM").is_ok_and(|term| term == "dumb")
    {
        eprintln!(
            "akasha: the TUI requires an interactive terminal and does not support --json; use a named command or --help"
        );
        return Err(2);
    }
    let request = ResolveRequest::from_process(root, project).map_err(|error| {
        eprintln!("akasha: {error}");
        error.exit_code()
    })?;
    let result = terminal_session(request, no_color, no_motion, ascii);
    result.map_err(|error| {
        eprintln!("akasha: terminal interface: {error}");
        6
    })
}

fn terminal_session(
    request: ResolveRequest,
    no_color: bool,
    no_motion: bool,
    ascii: bool,
) -> io::Result<()> {
    let state_path = state::state_path();
    let (restored_navigation, state_warning) = match state_path.as_deref().map(state::load) {
        Some(Ok(state)) => (state, None),
        Some(Err(error)) => (None, Some(error)),
        None => (None, None),
    };
    let _guard = TerminalGuard::new();
    enable_raw_mode()?;
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableFocusChange,
        EnableMouseCapture
    )?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    terminal.clear()?;
    let (jobs, responses) = app::worker();
    let mut app = App::new(
        request,
        jobs,
        no_motion,
        ascii,
        !no_color && std::env::var_os("NO_COLOR").is_none(),
    );
    if let Some(restored_navigation) = restored_navigation {
        app.restore_navigation(restored_navigation);
    }
    if let Some(warning) = state_warning {
        app.message(&format!(
            "Navigation persistence was ignored: {warning}. A clean exit will replace it."
        ));
    }
    app.load();
    let started = Instant::now();
    let mut redraw = true;
    let mut last_tick = Instant::now();
    let mut last_refresh_check = Instant::now();
    let mut focused = true;
    while !app.quit {
        match responses.try_recv() {
            Ok(result) => {
                redraw |= app.receive(result);
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err(io::Error::other("memory worker stopped unexpectedly"));
            }
        }
        if !app.no_motion && focused && last_tick.elapsed() >= Duration::from_millis(50) {
            app.sigil_tick = (started.elapsed().as_millis() / 50) as u64;
            if app.focus == app::Focus::Prompt || app.body_title == "WELCOME TO AKASHA" {
                app.tick = app.sigil_tick;
            }
            last_tick = Instant::now();
            redraw = true;
        }
        if focused && last_refresh_check.elapsed() >= AUTO_REFRESH_INTERVAL {
            app.check_external_changes();
            last_refresh_check = Instant::now();
        }
        if redraw {
            terminal.draw(|frame| view::draw(frame, &mut app))?;
            redraw = false;
        }
        if event::poll(Duration::from_millis(20))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => app.key(key),
                Event::Paste(text) => app.paste(&text),
                Event::Mouse(mouse) => app.mouse(mouse),
                Event::Resize(_, _) => {}
                Event::FocusGained => {
                    focused = true;
                    app.check_external_changes();
                    last_refresh_check = Instant::now();
                }
                Event::FocusLost => focused = false,
                _ => {}
            }
            redraw = true;
        }
    }
    if let (Some(path), Some(navigation)) = (state_path, app.navigation_state()) {
        state::save(&path, &navigation)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::TerminalGuard;
    use std::fs::{self, OpenOptions};
    use std::process::{Command, Stdio};

    #[test]
    fn panic_hook_restores_before_reporting() {
        if std::env::var_os("AKASHA_TUI_PANIC_TEST_CHILD").is_some() {
            let _guard = TerminalGuard::new();
            panic!("TUI panic probe");
        }

        let path = std::env::temp_dir().join(format!(
            "akasha-tui-panic-{}-{:?}.log",
            std::process::id(),
            std::thread::current().id()
        ));
        let output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let status = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tui::tests::panic_hook_restores_before_reporting",
                "--nocapture",
            ])
            .env("AKASHA_TUI_PANIC_TEST_CHILD", "1")
            .stdout(Stdio::from(output.try_clone().unwrap()))
            .stderr(Stdio::from(output))
            .status()
            .unwrap();
        let transcript = fs::read(&path).unwrap();
        fs::remove_file(path).unwrap();
        assert!(!status.success());
        let restored = transcript
            .windows(b"\x1b[?1049l".len())
            .position(|part| part == b"\x1b[?1049l")
            .expect("alternate screen must close");
        let diagnostic = transcript
            .windows(b"TUI panic probe".len())
            .position(|part| part == b"TUI panic probe")
            .expect("panic message must remain visible");
        assert!(restored < diagnostic);
        assert_eq!(
            transcript
                .windows(b"\x1b[?1049l".len())
                .filter(|part| *part == b"\x1b[?1049l")
                .count(),
            1,
            "panic and guard drop must restore only once"
        );
    }
}
