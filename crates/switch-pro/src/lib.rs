pub mod nfc;
mod spi;
use hex_literal::hex;
use nfc::{Mcu, Scan};
use std::time::{Duration, Instant};

pub const NFC_REPORT_LEN: usize = 363;

pub struct Controller {
    pub send_mtu: usize,
    pub mode: u8,
    pub info_seen: bool,
    pub lights_seen: bool,
    pub mcu: Mcu,
    address: [u8; 6],
    lights: u8,
    buttons: [u8; 3],
    release: Option<Instant>,
    imu: bool,
    start: Instant,
}
pub struct Frame {
    pub bytes: Vec<u8>,
    pub trailer: bool,
}
impl Controller {
    pub fn new(address: [u8; 6], send_mtu: usize) -> Self {
        Self {
            mode: 0x3f,
            send_mtu,
            info_seen: false,
            lights_seen: false,
            mcu: Mcu::default(),
            address,
            lights: 0,
            buttons: [0; 3],
            release: None,
            imu: false,
            start: Instant::now(),
        }
    }
    pub fn ready(&self) -> bool {
        self.info_seen && self.lights_seen && matches!(self.mode, 0x30 | 0x31)
    }
    pub fn press(&mut self, key: &str, now: Instant) -> Result<(), &'static str> {
        self.buttons = match key {
            "A" => [8, 0, 0],
            "B" => [4, 0, 0],
            "X" => [2, 0, 0],
            "Y" => [1, 0, 0],
            "Plus" => [0, 2, 0],
            "L+R" => [64, 0, 64],
            _ => return Err("Unknown button"),
        };
        self.release = Some(now + Duration::from_millis(180));
        Ok(())
    }
    fn standard(&self, report: u8, now: Instant) -> Vec<u8> {
        let mut b = vec![0; 50];
        b[..4].copy_from_slice(&[
            0xa1,
            report,
            (now.saturating_duration_since(self.start).as_millis() / 5) as u8,
            0x80,
        ]);
        b[4..7].copy_from_slice(&self.buttons);
        b[7..14].copy_from_slice(&hex!("6fc87716d87d80"));
        b
    }
    pub fn tick(&mut self, now: Instant) -> Frame {
        self.mcu.expire(now);
        if self.release.is_some_and(|t| now >= t) {
            self.buttons = [0; 3];
            self.release = None;
        }
        if self.mode == 0x3f {
            let [r, m, l] = self.buttons;
            return Frame {
                bytes: vec![
                    0xa1,
                    0x3f,
                    (r & 4) >> 2
                        | (r & 8) >> 2
                        | (r & 1) << 2
                        | (r & 2) << 2
                        | (l & 64) >> 2
                        | (r & 64) >> 1,
                    m,
                    8,
                    0,
                    128,
                    0,
                    128,
                    0,
                    128,
                    0,
                    128,
                ],
                trailer: false,
            };
        }
        let mut bytes = self.standard(self.mode, now);
        let mut trailer = false;
        if self.imu {
            for i in 0..3 {
                bytes[19 + i * 12] = 0x10;
            }
        }
        if self.mode == 0x31 {
            let (data, last) = self.mcu.frame();
            bytes.extend(data);
            trailer = last;
        }
        Frame { bytes, trailer }
    }

    pub fn output(&mut self, p: &[u8], now: Instant) -> Option<Vec<u8>> {
        self.mcu.expire(now);
        if p.len() < 12 || p[0] != 0xa2 {
            return None;
        }
        if p[1] == 0x11 {
            if self.mode == 0x31 {
                self.mcu.request(&p[11..]);
            }
            return None;
        }
        if p[1] != 1 {
            return None;
        }
        let op = p[11];
        let args = &p[12..];
        let mut b = self.standard(0x21, now);
        b[15] = op;
        b[14] = match (op, args) {
            (1 | 8, _) => 0x80,
            (2, _) => {
                self.info_seen = true;
                b[16..20].copy_from_slice(&[3, 0x48, 3, 2]);
                b[20..26].copy_from_slice(&self.address);
                b[26..28].copy_from_slice(&[1, 2]);
                0x82
            }
            (3, [mode @ (0x30 | 0x31 | 0x3f), ..])
                if *mode != 0x31 || self.send_mtu >= NFC_REPORT_LEN =>
            {
                if self.mode == 0x31 && *mode != 0x31 && self.mcu.scan == Scan::Reading {
                    self.mcu.remove(Scan::Cancelled);
                }
                if *mode == 0x31 && self.mode != 0x31 {
                    self.mcu.power(1);
                }
                self.mode = *mode;
                0x80
            }
            (4, _) => {
                b[16..25].copy_from_slice(&hex!("006a01bb0193019501"));
                0x83
            }
            (0x10, [a, b1, c, d, n, ..]) if *n <= 29 => {
                let base = u32::from_le_bytes([*a, *b1, *c, *d]);
                b[16..21].copy_from_slice(&args[..5]);
                for i in 0..*n {
                    b[21 + i as usize] = spi::read(base.wrapping_add(i as u32));
                }
                0x90
            }
            // The Classic Pro initialization fixture sends a short 0x21 probe
            // before NFC report mode. It expects a plain ACK, with no MCU payload.
            (0x21, [1, padding @ ..]) if self.mode != 0x31 && padding.iter().all(|b| *b == 0) => {
                0x80
            }
            (0x21, args) if self.mcu.configure(args) => {
                b[16..24].copy_from_slice(&hex!("0100ff0008001b01"));
                b[49] = nfc::crc8(&b[16..49]);
                0xa0
            }
            (0x22, [v, ..]) if self.mcu.power(*v) => 0x80,
            (0x30, [lights, ..]) => {
                self.lights = *lights;
                self.lights_seen = true;
                0x80
            }
            (0x31, _) => {
                b[16] = self.lights;
                0xb0
            }
            (0x40, [v, ..]) => {
                self.imu = *v != 0;
                0x80
            }
            (0x48, [_, ..]) => 0x80,
            _ => 0, // NACK unsupported requests, including SPI writes.
        };
        Some(b)
    }
}

pub mod mock;
#[cfg(test)]
mod tests;
