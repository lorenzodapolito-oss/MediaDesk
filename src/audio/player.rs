//! In-process playback (rodio + symphonia): real position, seeking, no external player.
//! Without the `playback` feature this is a stub that reports why it cannot play.

use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(feature = "playback")]
mod imp {
    use super::*;
    use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player};
    use std::fs::File;

    pub struct Playback {
        // Must outlive the player: dropping it closes the audio device.
        _sink: MixerDeviceSink,
        player: Player,
        file: PathBuf,
        /// Where the current decode started (seeking restarts the clock in some decoders).
        base: Duration,
    }

    impl Playback {
        pub fn start(path: &Path, from: Duration) -> Result<Self, String> {
            let sink =
                DeviceSinkBuilder::open_default_sink().map_err(|e| format!("audio device: {e}"))?;
            let player = Player::connect_new(sink.mixer());
            let file = File::open(path).map_err(|e| e.to_string())?;
            let decoder =
                Decoder::try_from(file).map_err(|e| format!("{}: {e}", path.display()))?;
            player.append(decoder);
            let mut base = Duration::ZERO;
            if from > Duration::ZERO && player.try_seek(from).is_ok() {
                base = Duration::ZERO;
            }
            Ok(Self {
                _sink: sink,
                player,
                file: path.to_path_buf(),
                base,
            })
        }
        pub fn file(&self) -> &Path {
            &self.file
        }
        pub fn position(&self) -> Duration {
            self.base + self.player.get_pos()
        }
        pub fn seek(&mut self, to: Duration) -> Result<(), String> {
            self.player.try_seek(to).map_err(|e| e.to_string())
        }
        pub fn pause(&self) {
            self.player.pause();
        }
        pub fn resume(&self) {
            self.player.play();
        }
        pub fn is_paused(&self) -> bool {
            self.player.is_paused()
        }
        pub fn finished(&self) -> bool {
            self.player.empty()
        }
        pub fn set_volume(&self, v: f32) {
            self.player.set_volume(v.clamp(0.0, 1.5));
        }
    }
}

#[cfg(not(feature = "playback"))]
mod imp {
    use super::*;
    pub struct Playback {
        file: PathBuf,
    }
    impl Playback {
        pub fn start(_: &Path, _: Duration) -> Result<Self, String> {
            Err("this build has no audio output".into())
        }
        pub fn file(&self) -> &Path {
            &self.file
        }
        pub fn position(&self) -> Duration {
            Duration::ZERO
        }
        pub fn seek(&mut self, _: Duration) -> Result<(), String> {
            Ok(())
        }
        pub fn pause(&self) {}
        pub fn resume(&self) {}
        pub fn is_paused(&self) -> bool {
            false
        }
        pub fn finished(&self) -> bool {
            true
        }
        pub fn set_volume(&self, _: f32) {}
    }
}

pub use imp::Playback;

/// `current ± delta` seconds, never below zero and never past `length` (when known).
pub fn seek_target(current: Duration, delta_secs: f64, length: Option<Duration>) -> Duration {
    let t = (current.as_secs_f64() + delta_secs).max(0.0);
    let t = match length {
        Some(l) if l.as_secs_f64() > 0.0 => t.min((l.as_secs_f64() - 0.05).max(0.0)),
        _ => t,
    };
    Duration::from_secs_f64(if t.is_finite() { t } else { 0.0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seek_is_clamped() {
        let l = Some(Duration::from_secs(100));
        assert_eq!(
            seek_target(Duration::from_secs(10), 30.0, l),
            Duration::from_secs(40)
        );
        assert_eq!(
            seek_target(Duration::from_secs(10), -30.0, l),
            Duration::ZERO
        );
        assert!(seek_target(Duration::from_secs(90), 30.0, l) < Duration::from_secs(100));
        assert_eq!(
            seek_target(Duration::from_secs(5), f64::NAN, None),
            Duration::ZERO
        );
    }
}
