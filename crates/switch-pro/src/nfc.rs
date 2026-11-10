use amiibo_core::Tag;
use hex_literal::hex;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub fn crc8(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0, |crc, byte| {
        (0..8).fold(crc ^ byte, |v, _| {
            if v & 0x80 == 0 {
                v << 1
            } else {
                (v << 1) ^ 7
            }
        })
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Scan {
    #[default]
    Idle,
    Armed,
    Discovered,
    Reading,
    Transferred,
    Cancelled,
    TimedOut,
    Unsupported,
    Failed,
}
impl Scan {
    pub fn active(self) -> bool {
        matches!(self, Self::Armed | Self::Discovered | Self::Reading)
    }
}

#[derive(Debug, Clone, Copy)]
enum Reply {
    Empty,
    Power,
    Poll,
    First,
    Second,
    Trailer,
}

pub struct Mcu {
    pub power: u8,
    pub scan: Scan,
    tag: Option<Tag>,
    deadline: Option<Instant>,
    polling: bool,
    seen: bool,
    reply: Reply,
}
impl Default for Mcu {
    fn default() -> Self {
        Self {
            power: 0,
            scan: Scan::Idle,
            tag: None,
            deadline: None,
            polling: false,
            seen: false,
            reply: Reply::Empty,
        }
    }
}

impl Mcu {
    pub fn arm(&mut self, tag: Tag, now: Instant) -> Result<(), &'static str> {
        if self.scan.active() {
            return Err("A tag is already armed");
        }
        self.tag = Some(tag);
        self.seen = false;
        self.reply = Reply::Empty;
        self.scan = Scan::Armed;
        self.deadline = Some(now + Duration::from_secs(60));
        Ok(())
    }
    pub fn remove(&mut self, outcome: Scan) {
        self.tag = None;
        self.seen = false;
        self.reply = Reply::Empty;
        self.deadline = None;
        self.scan = outcome;
    }
    pub fn expire(&mut self, now: Instant) {
        if self.deadline.is_some_and(|d| now >= d) {
            self.remove(Scan::TimedOut);
        }
    }
    pub fn power(&mut self, value: u8) -> bool {
        if value > 1 {
            return false;
        }
        if self.scan == Scan::Reading || (value == 0 && self.scan.active()) {
            self.remove(Scan::Cancelled);
        }
        self.power = value;
        self.polling = false;
        self.seen = false;
        self.reply = Reply::Power;
        true
    }
    pub fn configure(&mut self, args: &[u8]) -> bool {
        match args {
            [0x21, 0, 0, ..] => self.power(0),
            [0x21, 0, mode @ (1 | 4), ..] if self.power != 0 => {
                if self.scan == Scan::Reading {
                    self.remove(Scan::Cancelled);
                }
                self.power = *mode;
                self.polling = false;
                self.seen = false;
                self.reply = Reply::Power;
                true
            }
            _ => false,
        }
    }
    pub fn request(&mut self, args: &[u8]) {
        match args {
            [1, ..] if self.scan != Scan::Reading => self.reply = Reply::Power,
            [2, op, rest @ ..] if self.power == 4 => match op {
                0 | 2 => {
                    self.polling = false;
                    self.seen = false;
                    self.reply = Reply::Empty;
                    if self.scan == Scan::Reading || (*op == 0 && self.scan.active()) {
                        self.remove(Scan::Cancelled);
                    } else if self.scan == Scan::Discovered {
                        self.scan = Scan::Armed;
                    }
                }
                1 if self.scan != Scan::Reading => {
                    self.polling = true;
                    self.seen = false;
                    self.reply = Reply::Poll;
                }
                4 if self.scan != Scan::Reading => self.reply = Reply::Poll,
                6 if rest.len() < 13 => {
                    if self.scan.active() {
                        self.remove(Scan::Failed);
                    }
                }
                8 | 6 if *op == 8 || rest[6..13] != [0; 7] => self.remove(Scan::Unsupported),
                6 if self.polling && self.tag.is_some() && self.scan != Scan::Reading => {
                    self.scan = Scan::Reading;
                    self.reply = Reply::First;
                }
                _ => {}
            },
            _ => {}
        }
    }

    // One response per report tick. Status floods cannot overwrite a read segment.
    // The transport must confirm the trailer write before removing the tag.
    pub fn frame(&mut self) -> ([u8; 313], bool) {
        let mut b = [0u8; 313];
        let reply = std::mem::replace(&mut self.reply, Reply::Empty);
        let mut terminal = false;
        match (reply, self.tag.as_ref()) {
            (Reply::Power, _) if self.power != 0 => {
                b[..7].copy_from_slice(&hex!("0100000008001b"));
                b[7] = self.power;
            }
            (Reply::Poll, tag) => {
                b[..7].copy_from_slice(&hex!("2a000500000931"));
                b[7] = u8::from(self.polling);
                if let Some(tag) = tag.filter(|_| self.polling) {
                    b[7] = if self.seen { 9 } else { 1 };
                    self.seen = true;
                    b[8..16].copy_from_slice(&hex!("0000000101020007"));
                    b[16..23].copy_from_slice(&tag.uid);
                    self.scan = Scan::Discovered;
                }
            }
            (Reply::First, Some(tag)) => {
                b[..15].copy_from_slice(&hex!("3a0007010001310200000001020007"));
                b[15..22].copy_from_slice(&tag.uid);
                // NTAG metadata on the wire, documented in amiibo_edits/mcu.md.
                b[22..67].copy_from_slice(&hex!("000000007DFDF0793651ABD7466E39C191BABEB856CEEDF1CE44CC75EAFB27094D087AE803003B3C7778860000"));
                b[67..312].copy_from_slice(&tag.pages[..245]);
                self.reply = Reply::Second;
            }
            (Reply::Second, Some(tag)) => {
                b[..7].copy_from_slice(&hex!("3a000702000927"));
                b[7..302].copy_from_slice(&tag.pages[245..]);
                self.reply = Reply::Trailer;
            }
            (Reply::Trailer, Some(tag)) => {
                b[..16].copy_from_slice(&hex!("2a000500000931040000000101020007"));
                b[16..23].copy_from_slice(&tag.uid);
                terminal = true;
            }
            _ => b[0] = 0xff,
        }
        b[312] = crc8(&b[..312]);
        (b, terminal)
    }
}
