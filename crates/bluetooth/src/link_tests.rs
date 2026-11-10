use super::*;
use nix::sys::socket::{socketpair, AddressFamily, SockFlag, SockType};

fn pair() -> (SeqPacket, SeqPacket) {
    let (a, b) = socketpair(
        AddressFamily::Unix,
        SockType::SeqPacket,
        None,
        SockFlag::SOCK_NONBLOCK | SockFlag::SOCK_CLOEXEC,
    )
    .unwrap();
    // These test peers exercise seqpacket I/O and ownership only, without L2CAP-specific options.
    unsafe {
        (
            SeqPacket::from_raw_fd(a.into_raw_fd()).unwrap(),
            SeqPacket::from_raw_fd(b.into_raw_fd()).unwrap(),
        )
    }
}

#[tokio::test]
async fn fd_handoff_preserves_boundaries_and_releases_both_channels() {
    let (sender, receiver) = UnixStream::pair().unwrap();
    let (c, remote_c) = pair();
    let (i, remote_i) = pair();
    pass(&sender, &c, &i).await.unwrap();
    drop((sender, c, i));
    let (ctl, intr) = receive(&receiver).await.unwrap();
    remote_c.send(&[0x71]).await.unwrap();
    remote_i.send(&[0xa2, 1]).await.unwrap();
    let mut buf = [0; 64];
    assert_eq!(ctl.recv(&mut buf).await.unwrap(), 1);
    assert_eq!(buf[0], 0x71);
    assert_eq!(intr.recv(&mut buf).await.unwrap(), 2);
    assert_eq!(&buf[..2], &[0xa2, 1]);
    drop((ctl, intr));
    assert_eq!(remote_c.recv(&mut buf).await.unwrap(), 0);
    assert_eq!(remote_i.recv(&mut buf).await.unwrap(), 0);
}

#[tokio::test]
async fn closed_lease_without_descriptors_is_an_error() {
    let (sender, receiver) = UnixStream::pair().unwrap();
    drop(sender);
    assert!(receive(&receiver).await.is_err());
}
