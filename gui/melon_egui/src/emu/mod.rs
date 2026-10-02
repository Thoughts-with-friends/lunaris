//! The emulated console: one booted melonDS core and what it needs from us.
//!
//! # What lives here
//!
//! | file         | responsibility                                               |
//! |--------------|--------------------------------------------------------------|
//! | `console.rs` | [`Emu`]: boot a cart, run frames, savestates, save import   |
//! | `bridge.rs`  | the callbacks the core makes back (save data, stop, wifi)   |
//! | `cart.rs`    | the ROM header fields, and why a console stopped             |
//! | `clock.rs`   | the real-time clock (wall clock, or fixed for tests)         |
//!
//! Both consoles use this module: the first one on the UI thread
//! ([`crate::app`]) and the second one on its own thread ([`crate::guest`]).
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::SystemTime,
};

use melonds::Nds;

mod bridge;
mod cart;
mod clock;
mod console;

pub(crate) use bridge::{HostBridge, SaveSink};
pub use cart::{CartInfo, StopReason};
// Only the calendar tests reach for this directly.
#[cfg(test)]
pub(crate) use clock::civil_from_days;
pub use clock::{Clock, FRAME_RATE, use_deterministic_rtc, utc_clock};
pub(crate) use clock::{FIXED_RTC, deterministic_rtc, utc_now};
pub use console::{Emu, has_picture};

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use melonds::Host;

    use super::{HostBridge, SaveSink, civil_from_days};
    use crate::mp::Airwaves;

    /// A bridge on `seat`, or with no seat at all when `None`.
    fn bridge(seat: Option<crate::mp::Client>) -> HostBridge {
        HostBridge {
            saves: Arc::new(SaveSink {
                path: std::path::PathBuf::from("unused.sav"),
                pending: Mutex::new(None),
            }),
            stop: Arc::new(Mutex::new(None)),
            mp: seat,
            network: None,
        }
    }

    /// The bug local play failed on: a console booted without a seat has no
    /// way to join one later, because its `Host` is fixed when the core is
    /// constructed. Its frames used to vanish into the trait's defaults while
    /// the other console sat waiting for a host that could never be heard.
    #[test]
    fn a_console_with_a_seat_is_actually_on_the_air() {
        let air = Airwaves::new();
        let (host, guest) = (bridge(Some(air.client(0))), bridge(Some(air.client(1))));
        host.mp_begin();
        guest.mp_begin();

        host.mp_send_cmd(b"round", 1000);

        let mut buf = [0u8; 64];
        let mut ts = 0;
        assert_eq!(guest.mp_recv_host_packet(&mut buf, 0, &mut ts), Some(5));
        assert_eq!(&buf[..5], b"round");
        assert_eq!(air.counters()[0].sent_cmd, 1, "the console's CMD reached the medium");
    }

    #[test]
    fn a_console_without_a_seat_hears_nothing_and_is_heard_by_nobody() {
        let air = Airwaves::new();
        let guest = bridge(Some(air.client(1)));
        guest.mp_begin();
        // What `Emu::boot_with` builds: no seat at all.
        let seatless = bridge(None);

        // The trait's defaults claim the send succeeded, which is exactly why
        // this was invisible: nothing reports an error.
        assert_eq!(seatless.mp_send_cmd(b"round", 1000), 5);
        let mut buf = [0u8; 64];
        let mut ts = 0;
        assert_eq!(guest.mp_recv_packet(&mut buf, 0, &mut ts), Some(0), "nothing arrived");
        assert_eq!(air.counters()[0].sent_cmd, 0);
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        // 2000-02-29: a leap day in a century year that is also a leap year.
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(20_544), (2026, 4, 1));
    }
}
