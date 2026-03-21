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
    let args: &[&str] = &["pbcopy"];
    #[cfg(target_os = "linux")]
    let args: &[&str] = &["xclip", "-selection", "clipboard"];
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let args: &[&str] = &["pbcopy"]; // fallback

    let mut child = Command::new(args[0])
        .args(&args[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to spawn clipboard command: {}", e))?;

    // Take ownership of stdin and drop it after writing to send EOF
    {
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to open stdin for clipboard"))?;
        let mut stdin = stdin;
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| anyhow::anyhow!("Failed to write to clipboard: {}", e))?;
    } // stdin is dropped here, sending EOF

    let status = child.wait()?;
    if !status.success() {
        anyhow::bail!("Clipboard command exited with status: {}", status);
    }
    Ok(())
}
