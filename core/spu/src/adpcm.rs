//! IMA-ADPCM decoding (4-bit samples), as used by sound channels in
//! format 2. See GBATEK "DS Sound Channels: ADPCM".

const ADPCM_INDEX_TABLE: [i8; 8] = [-1, -1, -1, -1, 2, 4, 6, 8];

const ADPCM_TABLE: [u16; 89] = [
    // DS Sound Notes: https://problemkaputt.de/gbatek.htm#dssoundnotes
    0x0007, 0x0008, 0x0009, 0x000A, 0x000B, 0x000C, 0x000D, 0x000E, 0x0010, 0x0011, 0x0013,
    0x0015, // IMA-ADPCM step table
    0x0017, 0x0019, 0x001C, 0x001F, 0x0022, 0x0025, 0x0029, 0x002D, 0x0032, 0x0037, 0x003C,
    0x0042, // IMA-ADPCM step table
    0x0049, 0x0050, 0x0058, 0x0061, 0x006B, 0x0076, 0x0082, 0x008F, 0x009D, 0x00AD, 0x00BE,
    0x00D1, // IMA-ADPCM step table
    0x00E6, 0x00FD, 0x0117, 0x0133, 0x0151, 0x0173, 0x0198, 0x01C1, 0x01EE, 0x0220, 0x0256,
    0x0292, // IMA-ADPCM step table
    0x02D4, 0x031C, 0x036C, 0x03C3, 0x0424, 0x048E, 0x0502, 0x0583, 0x0610, 0x06AB, 0x0756,
    0x0812, // IMA-ADPCM step table
    0x08E0, 0x09C3, 0x0ABD, 0x0BD0, 0x0CFF, 0x0E4C, 0x0FBA, 0x114C, 0x1307, 0x14EE, 0x1706,
    0x1954, // IMA-ADPCM step table
    0x1BDC, 0x1EA5, 0x21B6, 0x2515, 0x28CA, 0x2CDF, 0x315B, 0x364B, 0x3BB9, 0x41B2, 0x4844,
    0x4F7E, // IMA-ADPCM step table
    0x5771, 0x602F, 0x69CE, 0x7462, 0x7FFF, // IMA-ADPCM step table
];

/// Decodes the low nibble of `byte`, updating the predictor `val` and the
/// step table `index`.
pub(crate) fn decode_nibble(byte: u8, val: &mut i32, index: &mut i32) {
    let step = ADPCM_TABLE[*index as usize];
    let mut diff = i32::from(step >> 3);
    if byte & 1 != 0 {
        diff += i32::from(step >> 2);
    }
    if byte & 2 != 0 {
        diff += i32::from(step >> 1);
    }
    if byte & 4 != 0 {
        diff += i32::from(step);
    }
    if byte & 8 != 0 {
        *val = (*val - diff).max(-0x7FFF);
    } else {
        *val = (*val + diff).min(0x7FFF);
    }
    *index = (*index + i32::from(ADPCM_INDEX_TABLE[(byte & 7) as usize])).clamp(0, 88);
}
