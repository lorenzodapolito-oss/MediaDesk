# MediaDesk

Audio and video in one program, a Rust rewrite of two Python tools (Audio Workstation Pro and a batch video converter).

* **Audio tab**: folder list with star ratings, tags / cover / lyrics preview, player (no VLC needed), autotag from
  MusicBrainz + iTunes + Deezer, rename from tags, MP3 160k conversion with loudness normalisation, recycle bin.
* **Video tab**: batch H.264/AAC conversion (MKV or MP4, max 1280 px wide), audio track choice, burned-in subtitles,
  colour correction, manual and automatic crop on a preview frame, per-file progress, stop button.

ffmpeg and ffprobe are used as external programs (there is no pure-Rust x264 or MP3 loudnorm). Put them next to the
program or in the PATH, or set the folder in Impostazioni.

```
cargo build --release
cargo test
```

Portable build on Windows: `packaging\windows\make-portable.ps1 [-FfmpegDir C:\ffmpeg\bin]`.

Licence: MIT OR Apache-2.0.
