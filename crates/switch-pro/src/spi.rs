use hex_literal::hex;

// Factory calibration and read-only identity from pcble2gamepad's Classic Pro profile.
pub fn read(address: u32) -> u8 {
    match address {
        0x6012 => 3,
        0x6013 => 0xa0,
        0x601b => 2,
        0x6020..=0x6037 => 0,
        0x603d..=0x604f => {
            hex!("baf5626fc877ed955b16d87df2b55f86655e0f")[(address - 0x603d) as usize]
        }
        0x6050..=0x6052 | 0x6056..=0x605b => 0x82,
        0x6053..=0x6055 => 15,
        0x605c..=0x6067 => 0,
        0x6080..=0x6097 => {
            hex!("5e010000f10f19d04cae40e1000000000000ffffffffffff")[(address - 0x6080) as usize]
        }
        0x6098..=0x60a9 => {
            hex!("19d04cae40e1000000000000ffffffffffff")[(address - 0x6098) as usize]
        }
        _ => 0xff,
    }
}
