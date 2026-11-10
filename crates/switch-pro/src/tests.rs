use super::*;
use crate::mock::{host, MockTransport};
use amiibo_core::Tag;

fn tag() -> Tag {
    let mut pages = [0; 540];
    for (i, b) in pages.iter_mut().enumerate() {
        *b = i as u8;
    }
    Tag {
        pages,
        uid: [4, 1, 2, 3, 4, 5, 6],
    }
}
#[test]
fn replay_initialization_and_read() {
    let now = Instant::now();
    let mut p = Controller::new([1, 2, 3, 4, 5, 6], NFC_REPORT_LEN);
    let mut wire = MockTransport::default();
    while let Some(input) = wire.incoming.pop_front() {
        let b = p.output(&input, now).unwrap();
        assert_eq!(&b[..2], &[0xa1, 0x21]);
        match input[11] {
            2 => assert_eq!(&b[16..28], &[3, 72, 3, 2, 1, 2, 3, 4, 5, 6, 1, 2]),
            0x10 => assert_eq!(&b[21..24], &[0xba, 0xf5, 0x62]),
            _ => {}
        }
    }
    assert!(p.ready());
    p.press("A", now).unwrap();
    assert_eq!(p.tick(now).bytes[4], 8);
    assert_eq!(p.tick(now + Duration::from_secs(1)).bytes[4], 0);
    p.mcu.arm(tag(), now).unwrap();
    wire.begin_scan().unwrap();
    assert!(wire.begin_scan().is_err());
    for _ in 0..8 {
        while let Some(input) = wire.incoming.pop_front() {
            p.output(&input, now);
        }
        let b = p.tick(now);
        wire.receive_input(&b.bytes).unwrap();
        if b.trailer {
            p.mcu.remove(Scan::Transferred);
            break;
        }
    }
    assert_eq!(p.mcu.scan, Scan::Transferred);
    assert_eq!(wire.received(), tag().pages);
}
#[test]
fn limited_mtu_keeps_standard_reports_available() {
    let now = Instant::now();
    let mut p = Controller::new([0; 6], 100);
    assert_eq!(p.output(&host(1, &[3, 0x30]), now).unwrap()[14], 0x80);
    assert_eq!(p.output(&host(1, &[3, 0x31]), now).unwrap()[14], 0);
    assert_eq!(p.tick(now).bytes.len(), 50);
}
#[test]
fn power_poll_errors_and_cancel() {
    let now = Instant::now();
    let mut m = Mcu::default();
    m.arm(tag(), now).unwrap();
    m.request(&[2, 1]);
    assert_eq!(m.frame().0[0], 0xff);
    m.power(1);
    assert_eq!(m.frame().0[..8], hex!("0100000008001b01"));
    assert!(m.configure(&[0x21, 0, 4]));
    m.request(&[2, 1]);
    let first = m.frame().0;
    assert_eq!(first[16..23], tag().uid);
    m.request(&[2, 4]);
    assert_eq!(m.frame().0[7], 9);
    m.request(&[2, 6, 0]);
    assert_eq!(m.scan, Scan::Failed);
    m.arm(tag(), now).unwrap();
    m.request(&[2, 8]);
    assert_eq!(m.scan, Scan::Unsupported);
    m.arm(tag(), now).unwrap();
    m.request(&[2, 0]);
    assert_eq!(m.scan, Scan::Cancelled);
    m.request(&[2, 4]);
    assert_eq!(m.frame().0[15], 0);
    m.arm(tag(), now).unwrap();
    m.expire(now + Duration::from_secs(60));
    assert_eq!(m.scan, Scan::TimedOut);
}
#[test]
fn malformed_host_and_crc_vector() {
    assert_eq!(nfc::crc8(b"123456789"), 0xf4);
    let now = Instant::now();
    let mut p = Controller::new([0; 6], NFC_REPORT_LEN);
    for len in 0..12 {
        assert!(p.output(&vec![0xa2; len], now).is_none());
    }
    for args in [
        &[3][..],
        &[0x10, 0, 0, 0, 0, 30],
        &[0x21, 0x21, 0, 4],
        &[0xab],
    ] {
        assert_eq!(p.output(&host(1, args), now).unwrap()[14], 0);
    }
    let probe = p.output(&host(1, &[0x21, 1]), now).unwrap();
    assert_eq!((probe[14], probe[49]), (0x80, 0));
    p.mcu.power(1);
    p.mcu.configure(&[0x21, 0, 4]);
    for op in 0..=255 {
        for len in 0..16 {
            let mut args = vec![2, op];
            args.resize(2 + len, 0);
            p.mcu.request(&args);
        }
    }
}

#[test]
fn removal_between_fragments_never_transmits_a_stale_second_half() {
    let now = Instant::now();
    let mut p = Controller::new([0; 6], NFC_REPORT_LEN);
    p.output(&host(1, &[3, 0x31]), now);
    let config = p.output(&host(1, &[0x21, 0x21, 0, 4]), now).unwrap();
    assert_eq!(config[49], 0xc8);
    p.mcu.arm(tag(), now).unwrap();
    p.output(&host(0x11, &[2, 1]), now);
    assert_eq!(p.tick(now).bytes[50], 0x2a);
    let mut read = vec![2, 6];
    read.resize(15, 0);
    p.output(&host(0x11, &read), now);
    assert_eq!(p.tick(now).bytes[53], 1);
    p.mcu.remove(Scan::Cancelled);
    let next = p.tick(now);
    assert_eq!(next.bytes[50], 0xff);
    assert!(!next.trailer);
    let mut other = tag();
    other.uid = [4, 3, 3, 3, 3, 3, 3];
    p.mcu.arm(other, now).unwrap();
    p.output(&host(0x11, &[2, 4]), now);
    assert_eq!(&p.tick(now).bytes[66..73], &[4, 3, 3, 3, 3, 3, 3]);
}
