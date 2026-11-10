#[cfg(target_os = "linux")]
mod bond;
#[cfg(target_os = "linux")]
mod linux;
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match std::env::args().nth(1).as_deref().unwrap_or("diagnose") {
        "diagnose" => println!(
            "{}",
            serde_json::to_string_pretty(&amiibo_bluetooth::diagnostic().await)?
        ),
        #[cfg(target_os = "linux")]
        "serve" => linux::serve().await?,
        #[cfg(target_os = "linux")]
        "recover" => linux::recover().await?,
        _ => anyhow::bail!("Usage: amiibo-bt diagnose | serve (Linux) | recover (Linux)"),
    };
    Ok(())
}
