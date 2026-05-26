#![cfg_attr(feature = "nightly", feature(core_intrinsics))]
#![expect(
    clippy::enum_variant_names,
    clippy::too_many_arguments,
    clippy::upper_case_acronyms
)]

#[cfg(feature = "nightly")]
use core::intrinsics::{likely, unlikely};

#[cfg(not(feature = "nightly"))]
use likely_stable::{likely, unlikely};

#[macro_use]
pub extern crate log;

use num_traits as num;
pub use simplelog;

mod arm;
mod hw;

pub mod nds;
pub use nds::NDS;
