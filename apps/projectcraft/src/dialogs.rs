//! Native Open, Save As and Export dialogs through `rfd`, without freezing the window.
//!
//! The dialog is built on the UI thread with the window as its parent (so it stays on top, and
//! macOS shows it as a sheet); a background thread waits for the answer and wakes the UI. Only
//! the UI's [`FileRequest`]s open a dialog, so nothing programmatic ever does.
//!
//! On Linux and the BSDs `rfd` asks the xdg-desktop-portal FileChooser and, failing that, runs
//! `zenity`; with neither it answers "cancelled" at once and nothing ever shows. So the waiting
//! thread first checks that one of them is there and otherwise answers "unavailable", and the app
//! shows its built-in file browser instead, as WordCraft does. `PROJECTCRAFT_FILE_DIALOG=builtin`
//! always uses the built-in one.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};

use eframe::egui;
use projectcraft_ui_egui::{App, EXTENSION, FileRequest};

/// A dialog's answer: the chosen path (`None` when cancelled), or no system dialog to show.
pub enum Answer {
    Picked(Option<PathBuf>),
    Unavailable,
}

/// The open dialog, if any: what was asked, and where its answer arrives.
#[derive(Default)]
pub struct Dialogs {
    waiting: Option<(FileRequest, Receiver<Answer>)>,
}

impl Dialogs {
    /// True while a dialog is open; another waits until it closes.
    pub fn busy(&self) -> bool {
        self.waiting.is_some()
    }

    /// The answer, once the dialog closes.
    pub fn answer(&mut self) -> Option<(FileRequest, Answer)> {
        let (request, rx) = self.waiting.as_ref()?;
        let answer = match rx.try_recv() {
            Ok(answer) => answer,
            Err(TryRecvError::Empty) => return None,
            // The waiting thread died: count it as cancelled.
            Err(TryRecvError::Disconnected) => Answer::Picked(None),
        };
        let request = *request;
        self.waiting = None;
        Some((request, answer))
    }

    pub fn show(&mut self, request: FileRequest, app: &App, parent: &eframe::Frame, ctx: &egui::Context) {
        let mut dialog = rfd::AsyncFileDialog::new().set_parent(parent);
        // Open also takes Microsoft Project XML and CSV, which it imports as a new project.
        dialog = match request {
            FileRequest::Open => dialog
                .add_filter("ProjectCraft project, Microsoft Project XML or CSV", &[EXTENSION, "xml", "csv"])
                .add_filter("ProjectCraft project", &[EXTENSION])
                .add_filter("Microsoft Project XML", &["xml"])
                .add_filter("CSV", &["csv"]),
            FileRequest::SaveAs => dialog.add_filter("ProjectCraft project", &[EXTENSION]),
            // A copy for another program: only its own kind of file is offered.
            FileRequest::Export(f) => dialog.add_filter(f.name(), &[f.extension()]),
        };
        if let Some(dir) = app.path.as_deref().and_then(Path::parent).filter(|d| d.is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        let picking: std::pin::Pin<Box<dyn Future<Output = Option<rfd::FileHandle>> + Send>> = match request {
            FileRequest::Open => Box::pin(dialog.pick_file()),
            FileRequest::SaveAs => Box::pin(dialog.set_file_name(app.save_as_name()).save_file()),
            FileRequest::Export(f) => Box::pin(dialog.set_file_name(app.export_name(f)).save_file()),
        };
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let spawned = thread::Builder::new().name("file dialog".into()).spawn(move || {
            // The picker only starts when first polled: without a system dialog it never is.
            let answer = match no_system_dialog() {
                Some(reason) => {
                    eprintln!("no system file dialog ({reason}): using the built-in one");
                    Answer::Unavailable
                }
                None => Answer::Picked(block_on(picking).map(|f| f.path().to_path_buf())),
            };
            let _ = tx.send(answer);
            ctx.request_repaint();
        });
        // If the thread couldn't start, its sender is gone and the answer reads as cancelled.
        if let Err(e) = spawned {
            eprintln!("couldn't wait for the file dialog: {e}");
        }
        self.waiting = Some((request, rx));
    }
}

/// Why no system file dialog can be shown, if it can't; run off the UI thread.
fn no_system_dialog() -> Option<String> {
    if std::env::var_os("PROJECTCRAFT_FILE_DIALOG").is_some_and(|v| v == "builtin") {
        return Some("PROJECTCRAFT_FILE_DIALOG=builtin".into());
    }
    portal_or_zenity_missing()
}

/// Linux and the BSDs: `rfd` needs a file chooser portal or zenity. When `dbus-send` can't ask
/// the session bus, the answer is unknown and `rfd` gets to try.
#[cfg(all(unix, not(target_os = "macos")))]
fn portal_or_zenity_missing() -> Option<String> {
    if on_path("zenity") {
        return None;
    }
    let portal = std::process::Command::new("dbus-send")
        .args([
            "--session",
            "--print-reply=literal",
            "--reply-timeout=10000",
            "--dest=org.freedesktop.portal.Desktop",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.DBus.Properties.Get",
            "string:org.freedesktop.portal.FileChooser",
            "string:version",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match portal {
        Ok(status) if !status.success() => Some("no xdg-desktop-portal FileChooser on the session bus and no zenity".into()),
        // Found, or the bus couldn't be asked (no `dbus-send`): let `rfd` try.
        _ => None,
    }
}

/// Windows and macOS always have their system dialog.
#[cfg(not(all(unix, not(target_os = "macos"))))]
fn portal_or_zenity_missing() -> Option<String> {
    None
}

/// Whether an executable named `program` is in a `PATH` folder.
#[cfg(all(unix, not(target_os = "macos")))]
fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
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

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn on_path_finds_a_shell_and_not_a_made_up_name() {
        assert!(on_path("sh"));
        assert!(!on_path("projectcraft-no-such-program"));
    }
}
