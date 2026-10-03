//! Find and start a Chromium-based browser with remote debugging enabled.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::CdpError;

#[derive(Debug, Clone)]
pub struct LaunchOptions {
    /// Explicit binary; otherwise `$BROWSE_CHROME`, then well-known names/paths.
    pub binary: Option<PathBuf>,
    pub headless: bool,
    /// Persistent profile directory; a temp dir is used when `None`.
    pub user_data_dir: Option<PathBuf>,
    pub window_size: (u32, u32),
    pub extra_args: Vec<String>,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self { binary: None, headless: true, user_data_dir: None, window_size: (1280, 900), extra_args: vec![] }
    }
}

impl LaunchOptions {
    pub fn headless() -> Self {
        Self::default()
    }

    pub fn headed() -> Self {
        Self { headless: false, ..Self::default() }
    }
}

pub struct ChromeProcess {
    pub ws_url: String,
    child: Child,
    _tmp: Option<tempfile::TempDir>,
}

impl Drop for ChromeProcess {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

/// Locate a Chromium-based browser binary.
pub fn find_browser() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("BROWSE_CHROME") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    let names = [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "chrome",
        "msedge",
        "microsoft-edge",
        "brave-browser",
    ];
    for n in names {
        if let Ok(p) = which::which(n) {
            return Some(p);
        }
    }
    let fixed = [
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    ];
    fixed.iter().map(PathBuf::from).find(|p| p.exists())
}

pub async fn launch(opts: LaunchOptions) -> Result<ChromeProcess, CdpError> {
    let binary = opts
        .binary
        .clone()
        .or_else(find_browser)
        .ok_or_else(|| CdpError::Launch("no Chromium-based browser found (set BROWSE_CHROME)".into()))?;
    let (tmp, data_dir) = match &opts.user_data_dir {
        Some(d) => (None, d.clone()),
        None => {
            let t = tempfile::Builder::new()
                .prefix("browse-profile-")
                .tempdir()
                .map_err(|e| CdpError::Launch(e.to_string()))?;
            let p = t.path().to_path_buf();
            (Some(t), p)
        }
    };

    let mut cmd = Command::new(&binary);
    cmd.arg("--remote-debugging-port=0")
        .arg(format!("--user-data-dir={}", data_dir.display()))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-background-networking")
        .arg("--disable-component-update")
        .arg("--disable-sync")
        .arg("--disable-default-apps")
        .arg("--disable-features=Translate,OptimizationHints,MediaRouter,DialMediaRouteProvider")
        .arg("--password-store=basic")
        .arg("--use-mock-keychain")
        .arg(format!("--window-size={},{}", opts.window_size.0, opts.window_size.1));
    if opts.headless {
        cmd.arg("--headless=new").arg("--hide-scrollbars");
    }
    let no_sandbox = std::env::var_os("BROWSE_CHROME_NO_SANDBOX").is_some()
        || std::env::var_os("CI").is_some()
        || (cfg!(unix) && is_root());
    if no_sandbox {
        cmd.arg("--no-sandbox");
    }
    for a in &opts.extra_args {
        cmd.arg(a);
    }
    cmd.arg("about:blank");
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| CdpError::Launch(format!("spawn {}: {e}", binary.display())))?;
    let stderr = child.stderr.take().ok_or_else(|| CdpError::Launch("no stderr".into()))?;
    let mut lines = BufReader::new(stderr).lines();
    let ws_url = tokio::time::timeout(Duration::from_secs(30), async {
        let mut tail = Vec::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(rest) = line.strip_prefix("DevTools listening on ") {
                return Ok(rest.trim().to_string());
            }
            tail.push(line);
            if tail.len() > 50 {
                tail.remove(0);
            }
        }
        Err(CdpError::Launch(format!("browser exited before DevTools was ready:\n{}", tail.join("\n"))))
    })
    .await
    .map_err(|_| CdpError::Launch("timed out waiting for DevTools endpoint".into()))??;

    // Keep draining stderr so the child never blocks on a full pipe.
    tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });

    Ok(ChromeProcess { ws_url, child, _tmp: tmp })
}

#[cfg(unix)]
fn is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("Uid:")).map(|l| l.contains("\t0\t")))
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_root() -> bool {
    false
}
