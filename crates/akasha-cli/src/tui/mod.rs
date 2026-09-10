mod app;
mod editor;
mod view;

use std::io::{self, IsTerminal};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use akasha_core::ResolveRequest;
use crossterm::{
    cursor::Show,
    event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use app::App;

/// Drop restores the terminal on normal exit and unwinding, including partial setup failure.
struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
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
    enable_raw_mode()?;
    let _guard = TerminalGuard;
    execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
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
    app.load();
    let started = Instant::now();
    let mut redraw = true;
    let mut last_tick = Instant::now();
    while !app.quit {
        match responses.try_recv() {
            Ok(result) => {
                app.receive(result);
                redraw = true;
            }
            Err(mpsc::TryRecvError::Empty) => {}
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err(io::Error::other("memory worker stopped unexpectedly"));
            }
        }
        if !app.no_motion
            && app.document.is_none()
            && last_tick.elapsed() >= Duration::from_millis(125)
        {
            app.tick = (started.elapsed().as_millis() / 125) as u64;
            last_tick = Instant::now();
            redraw = true;
        }
        if redraw {
            terminal.draw(|frame| view::draw(frame, &mut app))?;
            redraw = false;
        }
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => app.key(key),
                Event::Paste(text) => app.paste(&text),
                Event::Resize(_, _) => {}
                _ => {}
            }
            redraw = true;
        }
    }
    Ok(())
}
