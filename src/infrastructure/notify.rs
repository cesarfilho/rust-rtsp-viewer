//! Desktop notifications through `notify-send` (libnotify). Best-effort: if the
//! binary is missing the notification is simply dropped.

use std::process::{Command, Stdio};

pub fn send(title: &str, body: &str) {
    match Command::new("notify-send")
        .args(["--app-name=rust-rtsp-viewer", "--", title, body])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        // Reap the child so it does not linger as a zombie.
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => log::debug!("notify-send unavailable: {e}"),
    }
}

/// Open a folder in the desktop's file manager (`xdg-open`).
pub fn open_dir(dir: &std::path::Path) {
    match Command::new("xdg-open")
        .arg(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => log::warn!("xdg-open unavailable: {e}"),
    }
}
