use crate::{Notice, SOCKET};
use anyhow::{bail, ensure, Context, Result};
use bluer::l2cap::SeqPacket;
use nix::sys::socket::{recvmsg, sendmsg, ControlMessage, ControlMessageOwned, MsgFlags};
use std::{
    io::{self, IoSlice, IoSliceMut},
    os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, Interest},
    net::UnixStream,
};

pub async fn notice(s: &mut UnixStream, n: Notice) -> Result<()> {
    let data = serde_json::to_vec(&n)?;
    ensure!(data.len() <= 4096, "Oversized notice");
    s.write_u16(data.len() as u16).await?;
    s.write_all(&data).await?;
    Ok(())
}

pub async fn pass(s: &UnixStream, ctl: &SeqPacket, intr: &SeqPacket) -> Result<()> {
    let fds = [ctl.as_raw_fd(), intr.as_raw_fd()];
    let n = s
        .async_io(Interest::WRITABLE, || {
            sendmsg::<()>(
                s.as_raw_fd(),
                &[IoSlice::new(b"H")],
                &[ControlMessage::ScmRights(&fds)],
                MsgFlags::MSG_NOSIGNAL,
                None,
            )
            .map_err(io::Error::from)
        })
        .await?;
    ensure!(n == 1, "Failed to transfer connected channels");
    Ok(())
}

async fn receive(s: &UnixStream) -> Result<(SeqPacket, SeqPacket)> {
    let mut fds = s
        .async_io(Interest::READABLE, || {
            let mut byte = [0];
            let mut buf = [IoSliceMut::new(&mut byte)];
            let mut space = nix::cmsg_space!([i32; 2]);
            let msg = recvmsg::<()>(
                s.as_raw_fd(),
                &mut buf,
                Some(&mut space),
                MsgFlags::MSG_CMSG_CLOEXEC,
            )?;
            let mut fds = Vec::new();
            for msg in msg.cmsgs()? {
                if let ControlMessageOwned::ScmRights(raw) = msg {
                    // SCM_RIGHTS returns new, owned descriptors in this process.
                    fds.extend(
                        raw.into_iter()
                            .map(|fd| unsafe { OwnedFd::from_raw_fd(fd) }),
                    );
                }
            }
            Ok(fds)
        })
        .await?;
    ensure!(fds.len() == 2, "Helper did not supply both HID channels");
    let intr = fds.pop().unwrap();
    let ctl = fds.pop().unwrap();
    // Both descriptors were created as nonblocking L2CAP seqpacket sockets by the helper.
    unsafe {
        Ok((
            SeqPacket::from_raw_fd(ctl.into_raw_fd())?,
            SeqPacket::from_raw_fd(intr.into_raw_fd())?,
        ))
    }
}

pub struct Hid {
    lease: UnixStream,
    control_reply: Option<&'static [u8]>,
    mtu: usize,
    control: SeqPacket,
    interrupt: SeqPacket,
}
impl Hid {
    pub fn send_mtu(&self) -> usize {
        self.mtu
    }
    pub async fn connect(
        reconnect: bool,
        mut state: impl FnMut(Notice),
    ) -> Result<(Self, [u8; 6], String)> {
        let mut lease = UnixStream::connect(SOCKET)
            .await
            .context("connect to amiibo-bt.service")?;
        lease.write_u8(if reconnect { 2 } else { 1 }).await?;
        loop {
            let len = lease.read_u16().await? as usize;
            ensure!(len <= 4096, "Oversized helper notice");
            let mut bytes = vec![0; len];
            lease.read_exact(&mut bytes).await?;
            match serde_json::from_slice(&bytes)? {
                Notice::Connected { adapter, peer } => {
                    let (control, interrupt) = receive(&lease).await?;
                    return Ok((
                        Self {
                            lease,
                            control_reply: None,
                            mtu: interrupt.send_mtu()?,
                            control,
                            interrupt,
                        },
                        adapter,
                        peer,
                    ));
                }
                Notice::Error(reason) => bail!("{reason}"),
                n => state(n),
            }
        }
    }
    pub async fn recv(&mut self) -> Result<Vec<u8>> {
        let mut data = [0; 1024];
        let mut control = [0; 64];
        loop {
            // Keep a pending control response across cancellation by a report tick.
            if let Some(reply) = self.control_reply {
                ensure!(
                    self.control.send(reply).await? == reply.len(),
                    "Short HID control response"
                );
                self.control_reply = None;
            }
            tokio::select! {
                result = self.interrupt.recv(&mut data) => {
                    let n=result?;
                    ensure!(n>0 && n<data.len(),"HID interrupt closed or oversized packet");
                    return Ok(data[..n].to_vec());
                }
                n=self.control.recv(&mut control)=>{let n=n?;ensure!(n>0 && control[0]!=0x15,"HID control closed or virtual cable unplugged");
                    self.control_reply=Some(match control[0]{0x71=>&[0],0x60=>&[0xa0,1],_=>&[3]});
                }
                r=self.lease.readable()=>{r?;let mut b=[0];match self.lease.try_read(&mut b){Err(e) if e.kind()==io::ErrorKind::WouldBlock=>{},_=>bail!("Bluetooth helper closed the session")}}
            }
        }
    }
    pub async fn send(&self, b: &[u8]) -> Result<()> {
        ensure!(b.len() <= self.mtu, "HID report exceeds negotiated MTU");
        ensure!(
            self.interrupt.send(b).await? == b.len(),
            "Short L2CAP write"
        );
        Ok(())
    }
}

#[cfg(test)]
#[path = "link_tests.rs"]
mod tests;
