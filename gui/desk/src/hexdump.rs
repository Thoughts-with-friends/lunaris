//! Hexadecimal representation

/// Dump binary data in hexadecimal.
pub fn to_string<A>(bytes: A) -> String
where
    A: AsRef<[u8]>,
{
    let data = bytes.as_ref();
    let mut result = String::new();
    let mut offset = 0;

    for chunk in data.chunks(16) {
        // Print offset
        result.push_str(&format!("{:08x}: ", offset));
        offset += 16;

        // Print hex values
        for byte in chunk {
            result.push_str(&format!("{:02x} ", byte));
        }

        // Add padding for incomplete lines
        if chunk.len() < 16 {
            for _ in 0..(16 - chunk.len()) {
                result.push_str("   ");
            }
        }

        // Print ASCII representation
        result.push(' ');
        for byte in chunk {
            if byte.is_ascii_graphic() || *byte == b' ' {
                result.push(*byte as char);
            } else {
                result.push('.');
            }
        }
        result.push('\n');
    }

    result
}

/// Output binary data from hexdump.
pub fn to_bytes(hexdump: &str) -> Vec<u8> {
    let mut result = Vec::new();

    for line in hexdump.lines() {
        // Split line into offset, hex values, and ASCII representation
        if let Some(hex_part) = line.get(10..58) {
            for hex_byte in hex_part.split_whitespace() {
                if let Ok(byte) = u8::from_str_radix(hex_byte, 16) {
                    result.push(byte);
                }
            }
        }
    }

    result
}

/// Calculates the position in the hexdump output where the byte at the given
/// binary error position will appear.
///
/// The hexdump format for reference:
///
/// ```txt
/// 00000000: 48 65 6c 6c 6f 20 57 6f 72 6c 64 21 0a                  Hello World!
/// ```
///
/// In this format:
/// - The first 8 characters are the offset (`00000000`).
/// - The next 2 characters are a colon and a space (`: `).
/// - The next 48 characters are the hexadecimal representation of the 16 bytes of data(`48 65 6c 6c 6f 20 57 6f 72 6c 64 21 0a`).
/// - The last 16 characters are the ASCII representation of the 16 bytes of data (`Hello World!`).
///
/// Each line represents 16 bytes of the binary data.
pub const fn to_hexdump_pos(bytes_pos: usize) -> usize {
    const HEXDUMP_OFFSET: usize = 10;
    const BYTES_PER_LINE: usize = 16;
    const HEX_GROUP_SIZE: usize = 3;
    const ASCII_OFFSET: usize = 18;

    let line_number = bytes_pos / BYTES_PER_LINE;
    let line_offset = bytes_pos % BYTES_PER_LINE;

    HEXDUMP_OFFSET
        + (line_offset * HEX_GROUP_SIZE)
        + line_number * (HEXDUMP_OFFSET + (BYTES_PER_LINE * HEX_GROUP_SIZE) + ASCII_OFFSET)
}
