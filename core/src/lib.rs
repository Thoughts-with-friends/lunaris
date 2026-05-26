#![cfg_attr(feature = "nightly", feature(core_intrinsics))]
#![expect(clippy::duplicate_mod)]

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
