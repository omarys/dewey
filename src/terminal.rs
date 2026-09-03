use anyhow::{Context, Result};
use crossterm::{
    cursor::{Hide, Show},
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, Stdout};

pub type TuiTerminal = Terminal<CrosstermBackend<Stdout>>;

pub struct Tui {
    terminal: TuiTerminal,
}

impl Tui {
    pub fn new() -> Result<Self> {
        let terminal = Terminal::new(CrosstermBackend::new(io::stdout()))
            .context("Failed to create Ratatui terminal backend")?;
        Ok(Self { terminal })
    }

    pub fn init(&mut self) -> Result<()> {
        enable_raw_mode().context("Failed to enable raw mode")?;
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture, Hide)
            .context("Failed to enter alternate screen")?;
        self.terminal.clear().context("Failed to clear terminal")?;
        Ok(())
    }

    pub fn restore(&mut self) -> Result<()> {
        disable_raw_mode().ok();
        execute!(
            io::stdout(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            Show
        )
        .ok();
        self.terminal.show_cursor().ok();
        Ok(())
    }

    /// Temporarily suspends TUI mode so an external GUI or CLI app
    /// can run without interference.
    pub fn suspend(&mut self) -> Result<()> {
        disable_raw_mode().context("Failed to disable raw mode for suspension")?;
        execute!(
            io::stdout(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            Show
        )
        .context("Failed to leave alternate screen for suspension")?;
        Ok(())
    }

    /// Flushes any pending input events from the terminal to prevent keystroke leakage.
    pub fn flush_input(&mut self) -> Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            unsafe {
                libc::tcflush(std::io::stdin().as_raw_fd(), libc::TCIFLUSH);
            }
        }
        while crossterm::event::poll(std::time::Duration::from_millis(0)).unwrap_or(false) {
            let _ = crossterm::event::read();
        }
        Ok(())
    }

    /// Resumes TUI mode after an external process completes.
    pub fn resume(&mut self) -> Result<()> {
        enable_raw_mode().context("Failed to re-enable raw mode after suspension")?;
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture, Hide)
            .context("Failed to re-enter alternate screen after suspension")?;
        self.terminal
            .clear()
            .context("Failed to clear terminal upon resume")?;
        self.flush_input().ok();
        Ok(())
    }

    pub fn terminal_mut(&mut self) -> &mut TuiTerminal {
        &mut self.terminal
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}
