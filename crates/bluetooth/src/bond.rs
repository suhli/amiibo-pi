//! Capture only the link key notification for the peer owning both HID channels.
//! The private bluetoothd instance reloads this key on the next session.
use anyhow::{ensure, Result};
use bluer::Address;
use nix::libc;
use std::{
    fs, io,
    io::Write,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::Path,
};
use tokio::io::unix::AsyncFd;

#[repr(C)]
struct HciAddress {
    family: u16,
    device: u16,
    channel: u16,
}
pub struct Monitor(AsyncFd<OwnedFd>);
impl Monitor {
    pub fn open() -> Result<Self> {
        // Linux uapi bluetooth/hci.h: BTPROTO_HCI=1, HCI_DEV_NONE=0xffff, MONITOR=2.
        let raw = unsafe {
            libc::socket(
                libc::AF_BLUETOOTH,
                libc::SOCK_RAW | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
                1,
            )
        };
        if raw < 0 {
            return Err(io::Error::last_os_error().into());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let addr = HciAddress {
            family: libc::AF_BLUETOOTH as u16,
            device: 0xffff,
            channel: 2,
        };
        if unsafe {
            libc::bind(
                raw,
                (&addr as *const HciAddress).cast(),
                std::mem::size_of_val(&addr) as _,
            )
        } < 0
        {
            return Err(io::Error::last_os_error().into());
        }
        // The OwnedFd remains open and unchanged until AsyncFd drops it.
        Ok(Self(
            unsafe { AsyncFd::register(fd) }.map_err(|e| e.into_parts().1)?,
        ))
    }
    pub async fn next_key(&self, index: u16, host: Option<Address>) -> Result<Bond> {
        let mut packet = [0u8; 4096];
        loop {
            let n = self
                .0
                .async_io(tokio::io::Interest::READABLE, |fd| {
                    let n = unsafe {
                        libc::recv(fd.as_raw_fd(), packet.as_mut_ptr().cast(), packet.len(), 0)
                    };
                    if n < 0 {
                        Err(io::Error::last_os_error())
                    } else {
                        Ok(n as usize)
                    }
                })
                .await?;
            ensure!(n > 0, "HCI monitor closed");
            if let Some(key) = notification(&packet[..n], index, host) {
                return Ok(key);
            }
        }
    }
}

pub struct Bond {
    pub peer: Address,
    key: [u8; 16],
    kind: u8,
}
impl Bond {
    pub fn save(&self, local: Address) -> Result<()> {
        let dir = Path::new("/var/lib/amiibo-bt/bonds")
            .join(local.to_string())
            .join(self.peer.to_string());
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(dir.join("info.new"))?;
        writeln!(file,"[General]\nSupportedTechnologies=BR/EDR;\nTrusted=true\n\n[LinkKey]\nKey={}\nType={}\nPINLength=0",hex::encode_upper(self.key),self.kind)?;
        file.sync_all()?;
        fs::rename(dir.join("info.new"), dir.join("info"))?;
        fs::File::open(dir)?.sync_all()?;
        eprintln!("Classic bond saved for {}", self.peer);
        Ok(())
    }
}
fn notification(p: &[u8], index: u16, host: Option<Address>) -> Option<Bond> {
    if p.len() != 31
        || p[..2] != [3, 0]
        || p[2..4] != index.to_le_bytes()
        || p[4..8] != [25, 0, 0x18, 23]
    {
        return None;
    }
    let peer = Address([p[13], p[12], p[11], p[10], p[9], p[8]]);
    if host.is_some_and(|v| v != peer) {
        return None;
    }
    Some(Bond {
        peer,
        key: p[14..30].try_into().ok()?,
        kind: p[30],
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn monitor_filters_index_peer_and_lengths() {
        let mut p = [0; 31];
        p[..14].copy_from_slice(&[3, 0, 1, 0, 25, 0, 0x18, 23, 6, 5, 4, 3, 2, 1]);
        let peer = Address([1, 2, 3, 4, 5, 6]);
        assert!(notification(&p, 1, Some(peer)).is_some());
        assert!(notification(&p, 0, Some(peer)).is_none());
        assert!(notification(&p, 1, Some(Address([0; 6]))).is_none());
        assert!(notification(&p[..30], 1, Some(peer)).is_none());
    }
}
