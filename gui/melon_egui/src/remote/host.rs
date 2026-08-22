//! The machine both consoles run on.
//!
//! It streams the second console out and takes the remote player's controls
//! back. Everything else — saves, savestates, cheats, the instance directories
//! — stays here, which is the point of the mode.

use std::{
    io,
    net::{SocketAddr, UdpSocket},
    sync::{Arc, Mutex, atomic::Ordering},
};

use super::{
    RemoteStats, Tuning,
    audio::Downsampler,
    encoder::{Encoder, Pacer},
    session::{self, Session},
    wire::{self, Input},
};

/// How many sample pairs one audio datagram carries.
///
/// 240 pairs is 10 ms at the default 24 kHz transport rate, so a lost datagram
/// is a 10 ms gap — a click rather than a dropout — and the datagram stays well
/// inside the MTU.
const AUDIO_CHUNK_PAIRS: usize = 240;

/// The host end of a Remote Desktop session.
pub struct RemoteHost {
    session: Arc<Session>,
    /// The codec and its pacing decision, together: both are per-frame state
    /// and both are touched only from the streamed console's thread, so one
    /// lock covers them.
    video: Mutex<Video>,
    audio: Mutex<Downsampler>,
    tuning: Tuning,
}

struct Video {
    encoder: Encoder,
    pacer: Pacer,
    /// Reused between frames so encoding allocates nothing.
    datagrams: Vec<Vec<u8>>,
}

impl RemoteHost {
    /// Bind `bind_addr` and wait for one client.
    ///
    /// Blocks, so the caller runs it off the UI thread.
    ///
    /// # Errors
    /// If the port cannot be bound, or the socket fails while waiting.
    pub fn accept(bind_addr: SocketAddr, mut tuning: Tuning) -> io::Result<Self> {
        tuning.normalize();
        let socket = UdpSocket::bind(bind_addr)?;
        let client = session::accept_hello(&socket)?;
        let session = Session::start(socket, client, tuning, true)?;
        Ok(Self {
            session,
            video: Mutex::new(Video {
                encoder: Encoder::new(tuning.refresh_period),
                pacer: Pacer::new(&tuning),
                datagrams: Vec::new(),
            }),
            audio: Mutex::new(Downsampler::new()),
            tuning,
        })
    }

    /// Offer one frame to the stream.
    ///
    /// Called from the streamed console's own thread for **every** emulated
    /// frame; whether it is encoded is decided here by [`Pacer`]. Doing this on
    /// the UI thread would spend the other console's frame time on it, which is
    /// exactly the frame rate loss Remote Desktop exists to remove.
    ///
    /// A skipped frame costs nothing and delays nothing: its changes simply
    /// accumulate into the next frame that is sent.
    pub fn send_frame(&self, top: &[u32], bottom: &[u32]) {
        let counters = &self.session.counters;
        let mut video = self.video.lock().unwrap_or_else(|e| e.into_inner());
        if !video.pacer.due() {
            counters.bump(&counters.frames_skipped, 1);
            return;
        }

        let Video { encoder, pacer, datagrams } = &mut *video;
        let cost = encoder.encode(top, bottom, datagrams);
        for datagram in datagrams.iter() {
            if self.session.send(datagram) {
                counters.bump(&counters.video_datagrams, 1);
                counters.bump(&counters.video_bytes, datagram.len() as u64);
            }
        }
        pacer.observe(cost.bytes);

        counters.bump(&counters.frames, 1);
        counters.set(&counters.last_frame_tiles, cost.tiles as u64);
        counters.set(&counters.last_frame_bytes, cost.bytes as u64);
        counters.set(&counters.video_millifps, (pacer.frames_per_second() * 1000.0) as u64);
    }

    /// Put a frame's worth of the console's sound on the wire.
    ///
    /// `samples` is interleaved stereo `i16` at the console's own rate. It is
    /// decimated to [`Tuning::audio_rate`] on the way out and resampled back up
    /// by the client — see [`super::audio`] for why that is where the quality
    /// is kept.
    pub fn send_audio(&self, samples: &[i16]) {
        if !self.tuning.audio || samples.is_empty() {
            return;
        }
        let mut reduced = Vec::with_capacity(samples.len());
        self.audio.lock().unwrap_or_else(|e| e.into_inner()).run(
            samples,
            super::CONSOLE_SAMPLE_RATE,
            self.tuning.audio_rate,
            &mut reduced,
        );

        let counters = &self.session.counters;
        // Anything past the lag limit is dropped here rather than sent: a
        // datagram that arrives too late to play is bandwidth spent making the
        // sound later still.
        let limit = self.tuning.audio_backlog_pairs() * 2;
        let reduced = if reduced.len() > limit {
            counters.bump(&counters.audio_dropped, (reduced.len() - limit) as u64 / 2);
            &reduced[reduced.len() - limit..]
        } else {
            &reduced[..]
        };

        for chunk in reduced.chunks(AUDIO_CHUNK_PAIRS * 2) {
            let datagram =
                wire::encode_audio(self.session.next_seq(), self.tuning.audio_rate, chunk);
            if self.session.send(&datagram) {
                counters.bump(&counters.audio_pairs, chunk.len() as u64 / 2);
            }
        }
    }

    /// The remote player's current controls.
    ///
    /// Read afresh before **each** emulated frame rather than once per batch:
    /// a console catching up on several frames at once would otherwise apply
    /// one stale sample to all of them, which is felt as the stylus lagging
    /// behind the finger.
    #[must_use]
    pub fn input(&self) -> Input {
        self.session.input()
    }

    /// The address this end is bound to.
    ///
    /// # Errors
    /// If the socket cannot report it.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.session.local_addr()
    }

    /// The client that connected.
    #[must_use]
    pub fn remote_addr(&self) -> SocketAddr {
        self.session.remote
    }

    /// What the session is doing, for the diagnostics pane.
    #[must_use]
    pub fn stats(&self) -> RemoteStats {
        self.session.counters.snapshot(true)
    }
}

impl Drop for RemoteHost {
    fn drop(&mut self) {
        self.session.shutdown.store(true, Ordering::Relaxed);
    }
}
