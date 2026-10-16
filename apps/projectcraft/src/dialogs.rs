//! Native Open and Save As dialogs through `rfd`, without freezing the window.
//!
//! The dialog is built on the UI thread with the window as its parent (so it stays on top, and
//! macOS shows it as a sheet); a background thread waits for the answer and wakes the UI. Only
//! the UI's [`FileRequest`]s open a dialog, so nothing programmatic ever does.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};

use eframe::egui;
use projectcraft_ui_egui::{App, EXTENSION, FileRequest};

/// The open dialog, if any: what was asked, and where its answer arrives.
#[derive(Default)]
pub struct Dialogs {
    waiting: Option<(FileRequest, Receiver<Option<PathBuf>>)>,
}

impl Dialogs {
    /// True while a dialog is open; another waits until it closes.
    pub fn busy(&self) -> bool {
        self.waiting.is_some()
    }

    /// The answer, once the dialog closes: the chosen path, or `None` when cancelled.
    pub fn answer(&mut self) -> Option<(FileRequest, Option<PathBuf>)> {
        let (request, rx) = self.waiting.as_ref()?;
        let answer = match rx.try_recv() {
            Ok(path) => path,
            Err(TryRecvError::Empty) => return None,
            // The waiting thread died: count it as cancelled.
            Err(TryRecvError::Disconnected) => None,
        };
        let request = *request;
        self.waiting = None;
        Some((request, answer))
    }

    pub fn show(&mut self, request: FileRequest, app: &App, parent: &eframe::Frame, ctx: &egui::Context) {
        let mut dialog = rfd::AsyncFileDialog::new().set_parent(parent).add_filter("ProjectCraft project", &[EXTENSION]);
        if let Some(dir) = app.path.as_deref().and_then(Path::parent).filter(|d| d.is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        let picking: std::pin::Pin<Box<dyn Future<Output = Option<rfd::FileHandle>> + Send>> = match request {
            FileRequest::Open => Box::pin(dialog.pick_file()),
            FileRequest::SaveAs => {
                let name = app
                    .path
                    .as_deref()
                    .and_then(Path::file_name)
                    .map_or_else(|| format!("{}.{EXTENSION}", safe_name(&app.session.project.name)), |n| n.to_string_lossy().into_owned());
                Box::pin(dialog.set_file_name(name).save_file())
            }
        };
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let spawned = thread::Builder::new().name("file dialog".into()).spawn(move || {
            let _ = tx.send(block_on(picking).map(|f| f.path().to_path_buf()));
            ctx.request_repaint();
        });
        // If the thread couldn't start, its sender is gone and the answer reads as cancelled.
        if let Err(e) = spawned {
            eprintln!("couldn't wait for the file dialog: {e}");
        }
        self.waiting = Some((request, rx));
    }
}

/// A project name made fit for a file name: path separators and other characters some systems
/// refuse become spaces; an empty result is "Untitled".
fn safe_name(name: &str) -> String {
    let s: String = name.chars().take(100).map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { ' ' } else { c }).collect();
    let s = s.trim().trim_matches('.').trim();
    if s.is_empty() { "Untitled".into() } else { s.to_owned() }
}

/// Runs a future to completion on this thread (the dialog's wait), parking between polls.
fn block_on<F: Future>(f: F) -> F::Output {
    struct Unpark(Thread);
    impl Wake for Unpark {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }
    let waker = Waker::from(Arc::new(Unpark(thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut f = pin!(f);
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
        thread::park();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_name_strips_what_file_systems_refuse() {
        assert_eq!(safe_name("Office move"), "Office move");
        assert_eq!(safe_name("a/b\\c:d"), "a b c d");
        assert_eq!(safe_name("  ..  "), "Untitled");
        assert_eq!(safe_name(""), "Untitled");
        assert_eq!(safe_name(&"x".repeat(500)).len(), 100);
    }

    #[test]
    fn block_on_waits_for_a_wake_from_another_thread() {
        let (tx, rx) = channel::<u32>();
        let fut = std::future::poll_fn(move |cx| match rx.try_recv() {
            Ok(v) => Poll::Ready(v),
            Err(_) => {
                let w = cx.waker().clone();
                thread::spawn(move || w.wake());
                Poll::Pending
            }
        });
        let sender = thread::spawn(move || tx.send(7));
        assert_eq!(block_on(fut), 7);
        sender.join().unwrap().unwrap();
    }
}
