#![allow(dead_code)]

use crossterm::event::{Event as CrosstermEvent, EventStream, KeyEvent, MouseEvent};
use futures::{FutureExt, StreamExt};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::time::Interval;

use crate::scanner::ScanSummary;

#[derive(Debug, Clone)]
pub struct DownloadSuccessPayload {
    pub task_id: u64,
    pub series_id: i64,
    pub chapter_number: f64,
    pub file_path: PathBuf,
    pub page_count: Option<i64>,
    pub fetch_url: Option<String>,
    pub series_fetch_url: Option<String>,
}

#[derive(Debug, Clone)]
pub enum AppEvent {
    /// Terminal key press
    Key(KeyEvent),
    /// Terminal mouse event
    Mouse(MouseEvent),
    /// Terminal resize event (width, height)
    Resize(u16, u16),
    /// Periodic tick for animations (spinners, timers)
    Tick,
    /// Background library scan finished
    ScanCompleted(ScanSummary),
    /// Background Labrador download has started
    DownloadStarted {
        task_id: u64,
        series_id: i64,
        series_title: String,
        chapter_number: f64,
    },
    /// Background Labrador download finished successfully
    DownloadSuccess(DownloadSuccessPayload),
    /// Background Labrador download failed
    DownloadFailed {
        task_id: u64,
        series_id: i64,
        series_title: String,
        chapter_number: f64,
        error: String,
    },
    /// Flash message / notification toast
    Toast { message: String, is_error: bool },
    /// Quit the application
    Quit,
}

pub struct EventHandler {
    sender: UnboundedSender<AppEvent>,
    receiver: UnboundedReceiver<AppEvent>,
    handler_task: Option<tokio::task::JoinHandle<()>>,
    tick_rate: Duration,
}

impl EventHandler {
    pub fn new(tick_rate: Duration) -> (Self, UnboundedSender<AppEvent>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        let mut handler = Self {
            sender: sender.clone(),
            receiver,
            handler_task: None,
            tick_rate,
        };
        handler.resume();
        (handler, sender)
    }

    pub async fn next(&mut self) -> Option<AppEvent> {
        self.receiver.recv().await
    }

    /// Discards only user input events (such as stray keystrokes or mouse events typed during suspension).
    /// Preserves background task events (DownloadStarted, DownloadSuccess, DownloadFailed, ScanCompleted, Toast).
    pub fn drain(&mut self) {
        let mut preserved = Vec::new();
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                AppEvent::Key(_) | AppEvent::Mouse(_) | AppEvent::Tick => {}
                other => preserved.push(other),
            }
        }
        for event in preserved {
            let _ = self.sender.send(event);
        }
    }

    /// Suspends background input event streaming so an external child process
    /// (such as Labrador or Continuum) has exclusive, un-shared access to standard input.
    pub fn suspend(&mut self) {
        if let Some(task) = self.handler_task.take() {
            task.abort();
        }
        self.drain();
    }

    /// Resumes background input event streaming with a clean, newly opened EventStream.
    pub fn resume(&mut self) {
        self.drain();
        let tx = self.sender.clone();
        let tick_rate = self.tick_rate;

        let handler_task = tokio::spawn(async move {
            let mut reader = EventStream::new();
            let mut interval: Interval = tokio::time::interval(tick_rate);

            loop {
                let tick_delay = interval.tick();
                let crossterm_event = reader.next().fuse();

                tokio::select! {
                    _ = tick_delay => {
                        if tx.send(AppEvent::Tick).is_err() {
                            break;
                        }
                    }
                    Some(Ok(evt)) = crossterm_event => {
                        match evt {
                            CrosstermEvent::Key(key) if tx.send(AppEvent::Key(key)).is_err() => {
                                break;
                            }
                            CrosstermEvent::Mouse(mouse) if tx.send(AppEvent::Mouse(mouse)).is_err() => {
                                break;
                            }
                            CrosstermEvent::Resize(w, h) if tx.send(AppEvent::Resize(w, h)).is_err() => {
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }
        });

        self.handler_task = Some(handler_task);
    }
}
