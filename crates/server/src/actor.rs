use crate::state::*;
use anyhow::{bail, ensure, Result};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use switch_pro::{mock::MockTransport, nfc::Scan, Controller};
use tokio::{
    sync::mpsc,
    time::{interval, timeout, MissedTickBehavior},
};

enum Device {
    Mock(MockTransport),
    #[cfg(target_os = "linux")]
    Radio(amiibo_bluetooth::link::Hid),
}
impl Device {
    fn send_mtu(&self) -> usize {
        match self {
            Self::Mock(_) => switch_pro::NFC_REPORT_LEN,
            #[cfg(target_os = "linux")]
            Self::Radio(h) => h.send_mtu(),
        }
    }
    async fn open(app: &Shared, reconnect: bool) -> Result<(Self, [u8; 6], String)> {
        if app.simulation {
            app.phase(if reconnect {
                LinkState::Reconnecting
            } else {
                LinkState::Pairing
            });
            return Ok((
                Self::Mock(MockTransport::default()),
                [0x02, 0, 0, 0, 0, 1],
                "Simulation host".into(),
            ));
        }
        #[cfg(target_os = "linux")]
        {
            use amiibo_bluetooth::{link::Hid, Notice};
            let (hid, addr, peer) = Hid::connect(reconnect, |n| match n {
                Notice::Pairing => app.phase(LinkState::Pairing),
                Notice::Reconnecting => app.phase(LinkState::Reconnecting),
                _ => {}
            })
            .await?;
            Ok((Self::Radio(hid), addr, peer))
        }
        #[cfg(not(target_os = "linux"))]
        {
            bail!("Hardware mode requires Linux with BlueZ");
        }
    }
    async fn recv(&mut self) -> Result<Vec<u8>> {
        match self {
            Self::Mock(m) => {
                if let Some(b) = m.incoming.pop_front() {
                    Ok(b)
                } else {
                    std::future::pending().await
                }
            }
            #[cfg(target_os = "linux")]
            Self::Radio(h) => h.recv().await,
        }
    }
    async fn send(&mut self, b: &[u8]) -> Result<()> {
        match self {
            Self::Mock(m) => m.receive_input(b).map_err(anyhow::Error::msg),
            #[cfg(target_os = "linux")]
            Self::Radio(h) => timeout(Duration::from_secs(2), h.send(b)).await?,
        }
    }
    fn replay(&mut self) -> Result<()> {
        match self {
            Self::Mock(m) => m.begin_scan().map_err(anyhow::Error::msg),
            #[cfg(target_os = "linux")]
            Self::Radio(_) => bail!("Replay is available only in Simulation"),
        }
    }
}

fn snapshot(app: &App, p: &Controller) {
    let last = app.status.lock().unwrap().clone();
    if last.scan.active() && !p.mcu.scan.active() {
        if let Err(e) = app.db.lock().unwrap().record(
            last.armed_name.as_deref().unwrap_or(""),
            &format!("{:?}", p.mcu.scan),
            app.simulation,
        ) {
            app.log(e.to_string());
        }
    }
    app.modify(|s| {
        s.link = if p.ready() {
            LinkState::Ready
        } else if p.info_seen {
            LinkState::Initialized
        } else {
            LinkState::Connected
        };
        s.report = Some(p.mode);
        s.mtu = Some(p.send_mtu);
        s.scan = p.mcu.scan;
        s.power = p.mcu.power;
        if !s.scan.active() {
            s.armed_name = None;
        }
    });
}

fn action(app: &App, p: &mut Controller, d: &mut Device, input: Action) -> Result<()> {
    let now = Instant::now();
    match input {
        Action::Arm => {
            ensure!(
                p.send_mtu >= switch_pro::NFC_REPORT_LEN,
                "NFC requires a 363-byte MTU"
            );
            ensure!(p.ready(), "Controller has not finished initialization");
            let (name, tag) = app.db.lock().unwrap().selected()?;
            p.mcu.arm(tag, now).map_err(anyhow::Error::msg)?;
            match d {
                Device::Mock(m) => m.clear_scan(),
                #[cfg(target_os = "linux")]
                Device::Radio(_) => {}
            }
            app.modify(|s| s.armed_name = Some(name));
        }
        Action::Cancel => {
            ensure!(p.mcu.scan.active(), "No active tag");
            p.mcu.remove(Scan::Cancelled);
        }
        Action::Press(key) => {
            ensure!(p.ready(), "Controller is not ready");
            p.press(&key, now).map_err(anyhow::Error::msg)?;
        }
        Action::Replay => {
            ensure!(p.mcu.scan == Scan::Armed, "Arm a tag first");
            d.replay()?;
        }
        _ => bail!("A controller session is already active"),
    };
    snapshot(app, p);
    Ok(())
}

async fn session(app: &Shared, rx: &mut mpsc::Receiver<Request>, reconnect: bool) -> Result<()> {
    app.modify(|s| {
        s.link = LinkState::Preparing;
        s.error = None;
        s.peer = None;
        s.report = None;
        s.mtu = None;
        s.power = 0;
        s.armed_name = None;
        s.scan = Scan::Idle;
    });
    let open = timeout(Duration::from_secs(195), Device::open(app, reconnect));
    tokio::pin!(open);
    let (mut device, address, peer) = loop {
        tokio::select! {
            result=&mut open=>break result??,
            req=rx.recv()=>{let Some(req)=req else{return Ok(())};if matches!(req.action,Action::Disconnect){let _=req.response.send(Ok(()));return Ok(());}
                let _=req.response.send(Err("Connection is in progress".into()));}
        }
    };
    app.modify(|s| {
        s.link = LinkState::Connected;
        s.peer = Some(peer);
    });
    let mut p = Controller::new(address, device.send_mtu());
    let started = Instant::now();
    let mut count = 0u32;
    let mut next_trace = Instant::now();
    let mut tick = interval(Duration::from_micros(16667));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            pkt=device.recv()=>{
                let data=pkt?;
                let now=Instant::now();
                let response=p.output(&data,now);
                if now>=next_trace && data.len()>=12 {
                    let nack=response.as_ref().is_some_and(|r|r[14]==0);
                    if nack || app.debug.load(Ordering::Relaxed) {
                        app.log(format!("HID RX 0x{:02x}, command 0x{:02x}, {} bytes{}",data[1],data[11],data.len(),if nack{" (NACK)"}else{""}));
                        next_trace=now+Duration::from_secs(1);
                    }
                }
                if let Some(reply)=response {device.send(&reply).await?;}
            }
            _=tick.tick()=>{
                ensure!(p.ready() || started.elapsed()<Duration::from_secs(30),"No complete HID initialization within 30 seconds");
                let frame=p.tick(Instant::now());device.send(&frame.bytes).await?;
                if frame.trailer{p.mcu.remove(Scan::Transferred);}
                count=count.wrapping_add(1);if count.is_multiple_of(600) && app.debug.load(Ordering::Relaxed){app.log(format!("{count} input reports sent; mode 0x{:02x}",p.mode));}
            }
            req=rx.recv()=>{let Some(req)=req else{return Ok(())};if matches!(req.action,Action::Disconnect){let _=req.response.send(Ok(()));return Ok(());}
                let result=action(app,&mut p,&mut device,req.action).map_err(|e|e.to_string());let _=req.response.send(result);}
        }
        snapshot(app, &p);
    }
}

fn close(app: &Shared, result: &Result<()>) {
    let state = app.status.lock().unwrap().clone();
    let outcome = if result.is_err() {
        Scan::Failed
    } else {
        Scan::Cancelled
    };
    if state.scan.active() {
        if let Err(e) = app.db.lock().unwrap().record(
            state.armed_name.as_deref().unwrap_or(""),
            &format!("{outcome:?}"),
            app.simulation,
        ) {
            app.log(e.to_string());
        }
    }
    app.modify(|s| {
        s.link = if result.is_err() {
            LinkState::Error
        } else {
            LinkState::Disconnected
        };
        s.peer = None;
        s.report = None;
        s.mtu = None;
        s.power = 0;
        s.armed_name = None;
        if s.scan.active() {
            s.scan = outcome;
        }
        s.error = result.as_ref().err().map(|e| format!("{e:#}"));
    });
    if let Err(e) = result {
        app.log(format!("Connection: {e:#}"));
    }
}

pub async fn run(app: Shared, mut rx: mpsc::Receiver<Request>, autostart: bool) {
    let mut start = autostart.then_some(true);
    loop {
        let reconnect = match start.take() {
            Some(mode) => mode,
            None => {
                let Some(req) = rx.recv().await else {
                    break;
                };
                let mode = match req.action {
                    Action::Pair => false,
                    Action::Reconnect => true,
                    Action::Disconnect => {
                        let _ = req.response.send(Ok(()));
                        continue;
                    }
                    _ => {
                        let _ = req.response.send(Err("No controller session".into()));
                        continue;
                    }
                };
                let _ = req.response.send(Ok(()));
                mode
            }
        };
        let mut attempt = 0;
        let mut mode = reconnect;
        loop {
            let result = session(&app, &mut rx, mode).await;
            let retry = result.is_err()
                && (mode || app.status.lock().unwrap().peer.is_some())
                && !app.simulation;
            close(&app, &result);
            if !retry || attempt >= 5 {
                break;
            }
            mode = true;
            app.phase(LinkState::Reconnecting);
            let wait = tokio::time::sleep(Duration::from_secs(1 << attempt));
            tokio::pin!(wait);
            attempt += 1;
            let mut stop = false;
            loop {
                tokio::select! {
                    _=&mut wait=>break,
                    req=rx.recv()=>{let Some(req)=req else{return;};match req.action{
                        Action::Disconnect=>{stop=true;app.phase(LinkState::Disconnected);}
                        Action::Pair=>{start=Some(false);stop=true;}
                        Action::Reconnect=>{},
                        _=>{let _=req.response.send(Err("Reconnect pending".into()));continue;}
                    }let _=req.response.send(Ok(()));break;}
                }
            }
            if stop {
                break;
            }
        }
    }
}
