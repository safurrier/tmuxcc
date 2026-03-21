pub mod gh_client;
pub mod monitor;
pub mod types;

pub use gh_client::GhClient;
pub use types::*;

use std::io::Write;
use std::process::{Command, Stdio};

/// Open a URL in the default browser
pub fn open_url(url: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "linux")]
    let cmd = "xdg-open";
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let cmd = "open"; // fallback

    Command::new(cmd)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| anyhow::anyhow!("Failed to open URL: {}", e))
}

/// Copy text to the system clipboard
pub fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    let cmd = "pbcopy";
    #[cfg(target_os = "linux")]
    let cmd = "xclip";
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let cmd = "pbcopy"; // fallback

    let mut child = Command::new(cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to spawn clipboard command: {}", e))?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| anyhow::anyhow!("Failed to write to clipboard: {}", e))?;
    }

    child.wait()?;
    Ok(())
}
