mod app;
mod editor;
mod integration;
mod starlight;
mod state;
mod view;

use std::io::{self, IsTerminal};
use std::path::PathBuf;
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

/// Drop restores the terminal on normal exit and unwinding, including partial setup failure.
struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
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
    enable_raw_mode()?;
    let _guard = TerminalGuard;
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
