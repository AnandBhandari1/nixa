mod app;
mod brain;
mod cli;
mod view;

use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::{self, Stdout};
use std::process::ExitCode;
use std::time::Duration;

struct Screen {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

fn enter() -> io::Result<Screen> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    if let Err(err) = execute!(stdout, EnterAlternateScreen) {
        let _ = disable_raw_mode();
        return Err(err);
    }
    let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    Ok(Screen { terminal })
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

fn run(tree: &str, brain: Option<String>) -> Result<(), String> {
    let addr = brain::pick_addr(brain)?;
    let mut app = app::open(tree, &addr, &nixa_protocol::paths::data_dir())?;
    let mut screen = enter().map_err(|err| format!("Terminal error: {err}"))?;
    while !app.should_quit {
        screen
            .terminal
            .draw(|frame| view::draw(&mut app, frame))
            .map_err(|err| format!("Draw error: {err}"))?;
        if event::poll(Duration::from_millis(120))
            .map_err(|err| format!("Event poll error: {err}"))?
            && let Event::Key(key) =
                event::read().map_err(|err| format!("Event read error: {err}"))?
        {
            app.handle_key(key);
        }
    }
    Ok(())
}

pub fn run_main(args: Vec<String>) -> ExitCode {
    let result = match cli::parse(&args) {
        Ok(cli::Action::Help) => {
            println!("{}", cli::USAGE);
            Ok(())
        }
        Ok(cli::Action::Run { tree, brain }) => run(&tree, brain),
        Err(err) => Err(err),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("nixa-json: {err}");
            ExitCode::FAILURE
        }
    }
}
