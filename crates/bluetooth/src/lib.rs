use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::process::Command;

#[cfg(target_os = "linux")]
pub mod link;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Notice {
    Pairing,
    Reconnecting,
    Connected { adapter: [u8; 6], peer: String },
    Error(String),
}
pub const SOCKET: &str = "/run/amiibo-bt/control.sock";
pub fn adapter_name() -> String {
    std::env::var("AMIIBO_ADAPTER").unwrap_or_else(|_| "hci0".into())
}

pub async fn command(program: &str, args: &[&str]) -> anyhow::Result<String> {
    let mut cmd = Command::new(program);
    cmd.args(args).env("LC_ALL", "C").kill_on_drop(true);
    let out = tokio::time::timeout(Duration::from_secs(8), cmd.output()).await??;
    if !out.status.success() {
        anyhow::bail!(
            "{} {}: {} {}",
            program,
            out.status,
            String::from_utf8_lossy(&out.stderr).trim(),
            String::from_utf8_lossy(&out.stdout).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().into())
}

pub async fn diagnostic() -> serde_json::Value {
    let os = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| {
                l.strip_prefix("PRETTY_NAME=")
                    .map(|v| v.trim_matches('"').to_string())
            })
        });
    let adapter = adapter_name();
    let (version, service, info) = tokio::join!(
        command("bluetoothctl", &["--version"]),
        command("systemctl", &["is-active", "bluetooth"]),
        async {
            command(
                "btmgmt",
                &[
                    "--index",
                    adapter.strip_prefix("hci").unwrap_or(&adapter),
                    "info",
                ],
            )
            .await
        }
    );
    let adapters = std::fs::read_dir("/sys/class/bluetooth")
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    let classic = info
        .as_ref()
        .ok()
        .and_then(|s| s.lines().find(|l| l.contains("supported settings:")))
        .map(|line| line.split_whitespace().any(|s| s == "br/edr"));
    serde_json::json!({"os":os.unwrap_or_else(||std::env::consts::OS.into()),"arch":std::env::consts::ARCH,"adapter":adapter,"allowed_host":std::env::var("AMIIBO_HOST").ok(),"adapters":adapters,"classic":classic,"bluez":describe(version),"service":describe(service),"management":describe(info)})
}
fn describe(r: anyhow::Result<String>) -> String {
    r.unwrap_or_else(|e| format!("Unavailable: {e}"))
}
