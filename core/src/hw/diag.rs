//! Opt-in runtime diagnostics for rendering / audio triage.
//!
//! Every probe is gated on the `LUNARIS_DIAG` environment variable, which
//! holds a comma-separated list of probe names (or `all`). Nothing is printed
//! and no per-frame work is done unless a probe is requested, so release runs
//! are unaffected.
//!
//! Probe names match the diagnostics table in
//! `docs/design/rendering-audio-fix-design.md` §3:
//!
//! | Name      | Contents                                              |
//! |-----------|-------------------------------------------------------|
//! | `dispcnt` | D-1: per-frame DISPCNT + BGCNT for both 2D engines     |
//! | `vramcnt` | D-3: every VRAMCNT write (bank, MST, OFS, enable)      |
//! | `mosaic`  | D-4: every MOSAIC write (BG and OBJ sizes)             |
//! | `spu`     | D-5: SPU channel / SOUNDCNT writes                     |
//! | `capture` | D-6: DISPCAPCNT state at the start of each capture     |

use std::sync::OnceLock;

fn enabled_probes() -> &'static Vec<String> {
    static PROBES: OnceLock<Vec<String>> = OnceLock::new();
    PROBES.get_or_init(|| {
        std::env::var("LUNARIS_DIAG")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    })
}

/// Returns whether the named probe was requested via `LUNARIS_DIAG`.
pub fn probe(name: &str) -> bool {
    let probes = enabled_probes();
    probes.iter().any(|p| p == "all" || p == name)
}

/// Emits one diagnostic line for `name`, evaluating the arguments only when
/// that probe is enabled.
#[macro_export]
macro_rules! diag {
    ($name:literal, $($arg:tt)*) => {
        if $crate::hw::diag::probe($name) {
            eprintln!("[diag:{}] {}", $name, format_args!($($arg)*));
        }
    };
}
