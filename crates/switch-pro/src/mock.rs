//! Deterministic host-side test peer. It never opens Bluetooth sockets.
use crate::nfc::crc8;
use std::collections::VecDeque;
pub fn host(report: u8, args: &[u8]) -> Vec<u8> {
    let mut p = vec![0xa2, report];
    p.resize(11, 0);
    p.extend(args);
    p
}
pub struct MockTransport {
    pub incoming: VecDeque<Vec<u8>>,
    reading: bool,
    received: Vec<u8>,
}
impl Default for MockTransport {
    fn default() -> Self {
        Self {
            incoming: [
                vec![2],
                vec![0x10, 0x3d, 0x60, 0, 0, 18],
                vec![3, 0x30],
                vec![0x30, 1],
            ]
            .into_iter()
            .map(|p| host(1, &p))
            .collect(),
            reading: false,
            received: Vec::new(),
        }
    }
}
impl MockTransport {
    pub fn begin_scan(&mut self) -> Result<(), &'static str> {
        if self.reading {
            return Err("Simulation read is already in progress");
        }
        self.reading = true;
        self.received.clear();
        for args in [&[3, 0x31][..], &[0x22, 1], &[0x21, 0x21, 0, 4]] {
            self.incoming.push_back(host(1, args));
        }
        self.incoming.push_back(host(0x11, &[2, 1]));
        Ok(())
    }
    pub fn clear_scan(&mut self) {
        if self.reading {
            self.incoming.clear();
            self.reading = false;
        }
    }
    pub fn receive_input(&mut self, b: &[u8]) -> Result<(), &'static str> {
        if !self.reading || b.get(1) != Some(&0x31) {
            return Ok(());
        }
        if b.len() != 363 || crc8(&b[50..362]) != b[362] {
            return Err("Invalid MCU length or CRC");
        }
        let data = &b[50..];
        match (data[0], data[3], data[7]) {
            (0x2a, _, 1 | 9) if self.received.is_empty() && data[15] == 7 => {
                let mut read = vec![2, 6];
                read.resize(15, 0);
                self.incoming.push_back(host(0x11, &read));
            }
            (0x3a, 1, _) if self.received.is_empty() => self.received.extend(&data[67..312]),
            (0x3a, 2, _) if self.received.len() == 245 => self.received.extend(&data[7..302]),
            (0x3a, _, _) => return Err("Unexpected read segment"),
            (0x2a, _, 4) => {
                self.reading = false;
                if self.received.len() != 540 {
                    return Err("Incomplete tag read");
                }
            }
            _ => {}
        };
        Ok(())
    }
    pub fn received(&self) -> &[u8] {
        &self.received
    }
}
