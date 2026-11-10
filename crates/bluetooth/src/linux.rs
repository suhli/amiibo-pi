use amiibo_bluetooth::{
    adapter_name, command,
    link::{notice, pass},
    Notice, SOCKET,
};
use anyhow::{bail, ensure, Context, Result};
use bluer::{
    agent::Agent,
    l2cap::{Security, SecurityLevel, SeqPacket, Socket, SocketAddr},
    rfcomm::Profile,
    Adapter, Address, AddressType, Session,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    net::{UnixListener, UnixStream},
    time::timeout,
};

async fn bluez_mode(op: &str) -> Result<()> {
    let mut c = tokio::process::Command::new("/usr/local/libexec/amiibo-pi/bluez-mode");
    c.arg(op).kill_on_drop(true);
    let out = timeout(Duration::from_secs(45), c.output()).await??;
    ensure!(
        out.status.success(),
        "bluetoothd {op}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(())
}
async fn available(s: &Session, name: &str) -> Result<Adapter> {
    let adapter = s.adapter(name)?;
    timeout(Duration::from_secs(10), async {
        loop {
            if adapter.address().await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .context("BlueZ did not restore the selected adapter")?;
    Ok(adapter)
}
const DIR: &str = "/var/lib/amiibo-bt";
const JOURNAL: &str = "/var/lib/amiibo-bt/restore.json";
const PEER: &str = "/var/lib/amiibo-bt/peer.json";
#[derive(Serialize, Deserialize)]
struct Saved {
    name: String,
    address: String,
    alias: String,
    class: u32,
    powered: bool,
    discoverable: bool,
    pairable: bool,
    discovery_timeout: u32,
    pair_timeout: u32,
}

fn save<T: Serialize>(path: &str, value: &T) -> Result<()> {
    let tmp = format!("{path}.tmp");
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.sync_all()?;
    fs::rename(tmp, path)?;
    fs::File::open(DIR)?.sync_all()?;
    Ok(())
}
async fn set_class(adapter: &str, class: u32) -> Result<()> {
    command("hciconfig", &[adapter, "class", &format!("0x{class:06x}")]).await?;
    Ok(())
}
pub async fn recover() -> Result<()> {
    if !Path::new(JOURNAL).try_exists()? {
        return Ok(());
    }
    // Do not start a competing bluetoothd job during system shutdown.
    // The override lives in /run; the next boot replays the persistent journal.
    if command("systemctl", &["show", "--property=SystemState", "--value"])
        .await
        .as_deref()
        .ok()
        == Some("stopping")
    {
        eprintln!("System stopping; adapter restoration will resume at next boot");
        return Ok(());
    }
    let old: Saved = serde_json::from_slice(&fs::read(JOURNAL)?)?;
    if Path::new("/run/amiibo-bt/isolation/active").exists()
        || Path::new("/run/systemd/system/bluetooth.service.d/91-amiibo-pi.conf").exists()
    {
        bluez_mode("restore").await?;
    }
    let s = Session::new().await?;
    let a = available(&s, &old.name).await?;
    ensure!(
        a.address().await?.to_string() == old.address,
        "Adapter address changed; retaining recovery journal"
    );
    a.set_powered(true).await?;
    a.set_alias(old.alias).await?;
    a.set_pairable_timeout(old.pair_timeout).await?;
    a.set_discoverable_timeout(old.discovery_timeout).await?;
    a.set_pairable(old.pairable).await?;
    a.set_discoverable(old.discoverable).await?;
    set_class(&old.name, old.class).await?;
    a.set_powered(old.powered).await?;
    fs::remove_file(JOURNAL)?;
    Ok(())
}

fn socket(local: Address, psm: u16, security: SecurityLevel) -> Result<Socket<SeqPacket>> {
    let socket = Socket::new_seq_packet()?;
    socket.set_security(Security {
        level: security,
        key_size: 0,
    })?;
    socket
        .bind(SocketAddr::new(local, AddressType::BrEdr, psm))
        .with_context(|| {
            format!("bind {local} PSM {psm}; check BlueZ input plugin and other HID listeners")
        })?;
    Ok(socket)
}
async fn inbound(
    local: Address,
    a: &Adapter,
    name: &str,
    stream: &mut UnixStream,
    host: Option<Address>,
) -> Result<(SeqPacket, SeqPacket, Address)> {
    let ctl = socket(local, 17, SecurityLevel::Low)?.listen(1)?;
    let intr = socket(local, 19, SecurityLevel::Low)?.listen(1)?;
    a.set_pairable_timeout(180).await?;
    a.set_discoverable_timeout(180).await?;
    a.set_pairable(true).await?;
    a.set_discoverable(true).await?;
    set_class(name, 0x002508).await?;
    notice(stream, Notice::Pairing).await?;
    let ((ctl, c), (intr, i)) = tokio::try_join!(ctl.accept(), intr.accept())?;
    ensure!(c.addr == i.addr, "HID channels belong to different peers");
    ensure!(
        host.is_none_or(|addr| addr == c.addr),
        "Peer differs from AMIIBO_HOST"
    );
    Ok((ctl, intr, c.addr))
}
fn encrypted_peer(connections: &str, peer: Address) -> bool {
    connections.lines().any(|line| {
        let fields: Vec<_> = line.split_whitespace().collect();
        fields.get(1) == Some(&"ACL")
            && fields
                .get(2)
                .is_some_and(|v| v.eq_ignore_ascii_case(&peer.to_string()))
            && fields.contains(&"AUTH")
            && fields.contains(&"ENCRYPT")
    })
}
async fn authenticated(name: &str, peer: Address) -> Result<()> {
    timeout(Duration::from_secs(8), async {
        loop {
            if encrypted_peer(&command("hcitool", &["-i", name, "con"]).await?, peer) {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .context("No authenticated and encrypted Classic ACL for the HID peer")??;
    Ok(())
}
async fn outbound(local: Address, peer: Address) -> Result<(SeqPacket, SeqPacket, Address)> {
    let open = |psm| async move {
        socket(local, 0, SecurityLevel::Medium)?
            .connect(SocketAddr::new(peer, AddressType::BrEdr, psm))
            .await
            .map_err(anyhow::Error::from)
    };
    let ctl = open(17).await?;
    let intr = open(19).await?;
    Ok((ctl, intr, peer))
}

async fn lease(stream: &mut UnixStream, mode: u8) -> Result<()> {
    let name = adapter_name();
    ensure!(
        name.strip_prefix("hci")
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())),
        "Invalid adapter name"
    );
    let host = std::env::var("AMIIBO_HOST")
        .ok()
        .filter(|h| !h.is_empty())
        .map(|v| v.parse::<Address>())
        .transpose()
        .context("Invalid AMIIBO_HOST")?;
    let saved: Option<(String, String)> = if mode == 2 {
        Some(serde_json::from_slice(
            &fs::read(PEER).context("No saved host; pair first")?,
        )?)
    } else {
        None
    };
    let info = command("btmgmt", &["--index", &name[3..], "info"]).await?;
    ensure!(
        info.lines()
            .any(|l| l.contains("supported settings:")
                && l.split_whitespace().any(|v| v == "br/edr")),
        "Adapter does not advertise BR/EDR"
    );
    let session = Session::new().await?;
    let a = available(&session, &name).await?;
    let local = a.address().await?;
    let old = Saved {
        name: name.clone(),
        address: local.to_string(),
        alias: a.alias().await?,
        class: a.class().await?,
        powered: a.is_powered().await?,
        discoverable: a.is_discoverable().await?,
        pairable: a.is_pairable().await?,
        discovery_timeout: a.discoverable_timeout().await?,
        pair_timeout: a.pairable_timeout().await?,
    };
    save(JOURNAL, &old)?;
    let isolate = std::env::var("AMIIBO_ISOLATE").as_deref() != Ok("false");
    if isolate {
        bluez_mode("prepare").await?;
    }
    let session = Session::new().await?;
    let a = available(&session, &name).await?;
    ensure!(
        a.address().await? == local,
        "Adapter identity changed during preparation"
    );
    a.set_powered(true).await?;
    a.set_alias("Pro Controller".into()).await?;
    let monitor = if mode == 1 && isolate {
        Some(crate::bond::Monitor::open()?)
    } else {
        None
    };
    let profile = Profile {
        uuid: uuid::Uuid::from_u128(0x00001000_0000_1000_8000_00805f9b34fb),
        service_record: Some(include_str!("hid.xml").into()),
        require_authentication: Some(false),
        require_authorization: Some(false),
        ..Default::default()
    };
    let _sdp = session
        .register_profile(profile)
        .await
        .context("register HID SDP record")?;
    let agent = if mode == 1 {
        {
            // A default agent is global to bluetoothd. Limit its lifetime, controller and service.
            let confirm = name.clone();
            let authorize = name.clone();
            let service = name.clone();
            Some(
                session
                    .register_agent(Agent {
                        request_default: true,
                        request_confirmation: Some(Box::new(move |r| {
                            let ok = r.adapter == confirm && host.is_none_or(|v| v == r.device);
                            Box::pin(async move {
                                if ok {
                                    Ok(())
                                } else {
                                    Err(bluer::agent::ReqError::Rejected)
                                }
                            })
                        })),
                        request_authorization: Some(Box::new(move |r| {
                            let ok = r.adapter == authorize && host.is_none_or(|v| v == r.device);
                            Box::pin(async move {
                                if ok {
                                    Ok(())
                                } else {
                                    Err(bluer::agent::ReqError::Rejected)
                                }
                            })
                        })),
                        authorize_service: Some(Box::new(move |r| {
                            let ok = r.adapter == service
                                && host.is_none_or(|v| v == r.device)
                                && r.service.as_u128() == 0x00001124_0000_1000_8000_00805f9b34fb;
                            Box::pin(async move {
                                if ok {
                                    Ok(())
                                } else {
                                    Err(bluer::agent::ReqError::Rejected)
                                }
                            })
                        })),
                        ..Default::default()
                    })
                    .await?,
            )
        }
    } else {
        None
    };
    let index = name[3..].parse()?;
    let mut keys = std::collections::VecDeque::<crate::bond::Bond>::with_capacity(4);
    let (ctl, intr, peer) = if saved.is_none() {
        let pair = timeout(
            Duration::from_secs(180),
            inbound(local, &a, &name, stream, host),
        );
        tokio::pin!(pair);
        let result = if let Some(m) = &monitor {
            loop {
                tokio::select! {
                    channels=&mut pair=>break channels,
                    key=m.next_key(index,host)=>{
                        let key=key?;
                        keys.retain(|k|k.peer!=key.peer);
                        if keys.len()==4{keys.pop_front();}
                        keys.push_back(key);
                    }
                }
            }
        } else {
            pair.await
        };
        result.context("Pairing window expired")??
    } else {
        let (pair_adapter, peer) = saved.context("Missing saved peer")?;
        ensure!(
            host.is_none_or(|v| v.to_string().eq_ignore_ascii_case(&peer)),
            "Saved peer differs from AMIIBO_HOST"
        );
        ensure!(
            pair_adapter == local.to_string(),
            "Saved host belongs to another adapter"
        );
        a.set_pairable(false).await?;
        a.set_discoverable(false).await?;
        set_class(&name, 0x002508).await?;
        notice(stream, Notice::Reconnecting).await?;
        timeout(Duration::from_secs(15), outbound(local, peer.parse()?))
            .await
            .context("Reconnect timed out")??
    };
    authenticated(&name, peer).await?;
    let captured = keys.iter().find(|key| key.peer == peer);
    if let Some(key) = captured {
        key.save(local)?;
    }
    let awaiting_key = captured.is_none();
    drop(keys);
    a.set_discoverable(false).await?;
    a.set_pairable(false).await?;
    drop(agent);
    ensure!(
        intr.send_mtu()? >= 50,
        "Interrupt MTU is smaller than a standard Pro Controller report (50 bytes)"
    );
    save(PEER, &(local.to_string(), peer.to_string()))?;
    fs::write("/run/amiibo-bt/paired", b"")?;
    notice(
        stream,
        Notice::Connected {
            adapter: local.0,
            peer: peer.to_string(),
        },
    )
    .await?;
    pass(stream, &ctl, &intr).await?;
    // Outer task owns the disconnect watcher. Hold SDP, sockets and adapter until it fires.
    if let Some(m) = monitor {
        if awaiting_key {
            m.next_key(index, Some(peer)).await?.save(local)?;
        }
    }
    std::future::pending::<()>().await;
    Ok(())
}
async fn client(stream: UnixStream) -> Result<()> {
    let std = stream.into_std()?;
    let watcher = UnixStream::from_std(std.try_clone()?)?;
    let mut stream = UnixStream::from_std(std)?;
    let mode = timeout(Duration::from_secs(5), stream.read_u8()).await??;
    ensure!(matches!(mode, 1 | 2), "Unknown helper operation");
    let result = tokio::select! {
        work=lease(&mut stream,mode)=>work,
        result=async {let mut b=[0];loop{watcher.readable().await?;match watcher.try_read(&mut b){
            Ok(0)=>return Ok(()),Ok(_)=>bail!("Unexpected client data"),Err(e) if e.kind()==std::io::ErrorKind::WouldBlock=>continue,Err(e)=>return Err(e.into())
        }}}=>result,
    };
    if let Err(e) = &result {
        let _ = notice(&mut stream, Notice::Error(format!("{e:#}"))).await;
    }
    result
}

pub async fn serve() -> Result<()> {
    recover().await?;
    if Path::new(SOCKET).try_exists()? {
        fs::remove_file(SOCKET)?;
    }
    let diagnostic_path = "/run/amiibo-bt/diagnostic.json";
    fs::write(
        diagnostic_path,
        serde_json::to_vec(&amiibo_bluetooth::diagnostic().await)?,
    )?;
    fs::set_permissions(diagnostic_path, fs::Permissions::from_mode(0o640))?;
    if Path::new(PEER).try_exists()? {
        fs::write("/run/amiibo-bt/paired", b"")?;
    }
    let listener = UnixListener::bind(SOCKET)?;
    fs::set_permissions(SOCKET, fs::Permissions::from_mode(0o660))?;
    if let Some(path) = std::env::var_os("NOTIFY_SOCKET") {
        use std::os::{
            linux::net::SocketAddrExt,
            unix::{
                ffi::OsStrExt,
                net::{SocketAddr, UnixDatagram},
            },
        };
        let destination = match path.as_bytes().strip_prefix(b"@") {
            Some(name) => SocketAddr::from_abstract_name(name)?,
            None => SocketAddr::from_pathname(path)?,
        };
        UnixDatagram::unbound()?.send_to_addr(b"READY=1", &destination)?;
    }
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    loop {
        let stream = tokio::select! {s=listener.accept()=>s?.0,_=term.recv()=>break,_=tokio::signal::ctrl_c()=>break};
        let stop = tokio::select! {result=client(stream)=>{if let Err(e)=result{eprintln!("Bluetooth session: {e:#}");}false},_=term.recv()=>true,_=tokio::signal::ctrl_c()=>true};
        recover()
            .await
            .context("Adapter restoration failed; journal retained")?;
        if stop {
            break;
        }
    }
    fs::remove_file(SOCKET)?;
    Ok(())
}
