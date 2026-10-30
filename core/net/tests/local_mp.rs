//! Two Wi-Fi controllers on melonDS's LocalMP medium (real FFI path): a
//! frame one transmits must land in the other's RX buffer.

use ds_net::{MpBackend, NetLink};
use ds_peripherals::{spi::firmware::Firmware, wifi::WiFi};

const IO: u32 = 0x0480_0000;
/// ARM7 cycles in 5 ms.
const FIVE_MS: u32 = 33_513_982 / 200;

fn write_rf(wifi: &mut WiFi, id: u8, value: u8) {
    // DS Wifi RF Chip: https://problemkaputt.de/gbatek.htm#dswifirfchip
    wifi.write(IO + 0x17E, (u16::from(id) << 8) | u16::from(value)); // W_RF_DATA1 (480817Eh): register index + value
    wifi.write(IO + 0x17C, 5); // W_RF_DATA2 (480817Ch): start transfer
}

/// Powered, on channel 1 (the built-in firmware's RF type 3 tables), MAC
/// `00:09:BF:00:00:<last>`, RX into 0x4C00..0x5F00 with every filter open.
fn console(instance: usize, last: u8) -> WiFi {
    let mut fw = Firmware::new();
    fw.load(&ds_bios::firmware::FIRMWARE_DS);
    let mut wifi = WiFi::new();
    wifi.reset(&fw);
    wifi.set_link(Some(Box::new(NetLink::new(instance).with_pump(instance == 0))));
    wifi.set_power_cnt(true);
    // DS Wifi I/O Map: https://problemkaputt.de/gbatek.htm#dswifiiomap
    wifi.write(IO + 0x036, 0); // W_POWER_US (4808036h): power up

    write_rf(&mut wifi, 0x01, 0x4B); // RF register 01h: channel data
    write_rf(&mut wifi, 0x02, 0x6C); // RF register 02h: channel data
    assert_eq!(wifi.channel(), 1);

    for (i, half) in [0x0900_u16, 0x00BF, u16::from(last) << 8] // MAC 00:09:BF:00:00:<last> as three halfwords
        .into_iter()
        .enumerate()
    {
        wifi.write(IO + 0x018 + 2 * i as u32, half); // W_MACADDR_0..2 (4808018h)
    }
    wifi.write(IO + 0x050, 0x4C00); // W_RXBUF_BEGIN (4808050h): Wifi RAM 4804C00h
    wifi.write(IO + 0x052, 0x5F00); // W_RXBUF_END (4808052h): Wifi RAM 4805F00h
    wifi.write(IO + 0x056, 0x0C00 >> 1); // W_RXBUF_WR_ADDR (4808056h) in halfwords
    wifi.write(IO + 0x0D0, 0xFFFF); // W_RXFILTER (48080D0h): accept everything
    wifi.write(IO + 0x0E0, 0); // W_RXFILTER2 (48080E0h)
    wifi.write(IO + 0x030, 0x8001); // W_RXCNT (4808030h): receive enable + latch write address
    wifi
}

#[test]
fn frame_crosses_local_mp() {
    ds_net::init(MpBackend::Local);
    let mut host = console(0, 1);
    let mut guest = console(1, 2);

    // Broadcast data frame with a 4-byte body, TX header at Wi-Fi RAM 0.
    let length: u16 = 24 + 4 + 4;
    let mut frame = vec![0_u8; 12];
    // DS Wifi Hardware Headers: https://problemkaputt.de/gbatek.htm#dswifihardwareheaders
    frame[8] = 0x14; // TX header +08h: rate 2 Mbit/s
    frame[0xA..0xC].copy_from_slice(&length.to_le_bytes()); // TX header +0Ah: frame length
    frame.extend_from_slice(&[0x08, 0x00, 0, 0]); // 802.11 frame control: data, duration 0
    frame.extend_from_slice(&[0xFF; 6]); // broadcast destination
    frame.extend_from_slice(&[0x00, 0x09, 0xBF, 0x00, 0x00, 0x01]); // source MAC 00:09:BF:00:00:01
    frame.extend_from_slice(&[0x00, 0x09, 0xBF, 0x00, 0x00, 0x01]); // BSSID 00:09:BF:00:00:01
    frame.extend_from_slice(&[0, 0, 0x12, 0x34, 0x56, 0x78, 0, 0, 0, 0]); // sequence control + test payload
    for (i, pair) in frame.chunks(2).enumerate() {
        host.write(
            IO + 0x4000 + 2 * i as u32, // Wifi RAM 4804000h
            u16::from_le_bytes([pair[0], pair[1]]),
        );
    }
    host.write(IO + 0x0A0, 0x8000); // W_TXBUF_LOC1
    host.write(IO + 0x0AE, 1); // W_TXREQ_SET

    host.new_frame();
    host.run(FIVE_MS);
    guest.new_frame();
    guest.run(FIVE_MS);

    assert_ne!(guest.read(IO + 0x010) & 1, 0, "RX IRQ"); // W_IF (4808010h) bit 0: RX complete IRQ
    assert_eq!(guest.read(IO + 0x4C00 + 12), 0x0008, "frame control"); // 802.11 frame control after the 12-byte RX header
    assert_eq!(guest.read(IO + 0x4C00 + 12 + 24), 0x3412, "body"); // frame body (test payload)
    assert_eq!(guest.read(IO + 0x4C00 + 12 + 10), 0x0900, "sender MAC"); // sender MAC first halfword

    // Powering Wi-Fi off leaves the medium (MP_End) without complaint.
    host.set_power_cnt(false);
    guest.set_power_cnt(false);
}
