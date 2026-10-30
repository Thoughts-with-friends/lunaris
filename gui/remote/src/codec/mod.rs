//! What crosses the wire, as bytes: the picture (565 tiles, encoder and
//! decoder) and the sound (downsampled for transport).

pub mod audio;
pub mod colour;
pub mod decoder;
pub mod encoder;
pub mod tile;
