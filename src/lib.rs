//! MediaDesk: an audio tagger / player / converter and a batch video converter, both front ends
//! for ffmpeg. The logic lives in these modules; `ui` draws it.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![forbid(unsafe_code)]

pub mod audio;
pub mod ffmpeg;
pub mod settings;
pub mod theme;
pub mod ui;
pub mod util;
pub mod video;
