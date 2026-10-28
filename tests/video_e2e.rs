//! Runs the real ffmpeg/ffprobe on a generated clip (skipped when they are not installed).

use mediadesk::ffmpeg::{self, find_tool, new_cancel};
use mediadesk::video::convert::{self, AudioChoice, Container, Options};
use mediadesk::video::filters::Crop;
use mediadesk::video::probe;
use std::time::Duration;

#[test]
fn convert_a_generated_clip_with_crop_and_colour() {
    let (Some(ffmpeg_bin), Some(ffprobe_bin)) = (find_tool("ffmpeg", ""), find_tool("ffprobe", ""))
    else {
        eprintln!("skipped: ffmpeg/ffprobe not installed");
        return;
    };
    let dir = std::env::temp_dir().join(format!("mediadesk-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("clip.mkv");
    let make: Vec<String> = [
        "-nostdin",
        "-y",
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=1920x1080:rate=25:duration=2",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=2",
        "-c:v",
        "libx264",
        "-c:a",
        "aac",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain(std::iter::once(src.to_string_lossy().into_owned()))
    .collect();
    let (_, err) = ffmpeg::capture(&ffmpeg_bin, &make, Duration::from_secs(60)).unwrap();
    assert!(src.exists(), "{err}");

    let (out, _) =
        ffmpeg::capture(&ffprobe_bin, &probe::args(&src), Duration::from_secs(30)).unwrap();
    let pr = probe::parse(&out).unwrap();
    assert_eq!((pr.width, pr.height), (1920, 1080));
    assert_eq!(pr.audio.len(), 1);

    // without ffprobe the same streams come from `ffmpeg -i`
    let alt = probe::read(None, Some(&ffmpeg_bin), &src).unwrap();
    assert_eq!((alt.width, alt.height, alt.audio.len()), (1920, 1080, 1));
    assert!((alt.duration - pr.duration).abs() < 0.2);

    let mut o = Options {
        container: Container::Mp4,
        audio: AudioChoice::All,
        crop: Some(
            Crop {
                left: 160,
                right: 160,
                top: 0,
                bottom: 0,
            }
            .clamped(1920, 1080),
        ),
        ..Default::default()
    };
    o.color.enabled = true;
    o.color.saturation = 1.4;
    o.color.temperature = 20;
    let target = convert::output_path(&src, &dir, o.container);
    let args = convert::args(&src, &target, &o, &pr);
    let mut last = 0.0;
    let res = ffmpeg::run(&ffmpeg_bin, &args, &new_cancel(), |s| last = s).unwrap();
    assert!(res.success, "{}", res.stderr_tail);
    assert!(last > 0.0, "no progress reported");

    let (out, _) =
        ffmpeg::capture(&ffprobe_bin, &probe::args(&target), Duration::from_secs(30)).unwrap();
    let done = probe::parse(&out).unwrap();
    // 1920-320 = 1600 wide, scaled down to 1280; 1600x1080 keeps its ratio: 1280x864
    assert_eq!(done.width, 1280);
    assert_eq!(done.height, 864);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cancelling_stops_ffmpeg_and_reports_it() {
    let Some(ffmpeg_bin) = find_tool("ffmpeg", "") else {
        return;
    };
    let cancel = new_cancel();
    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    let args: Vec<String> = [
        "-nostdin",
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=640x360:rate=25:duration=600",
        "-f",
        "null",
        "-",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let res = ffmpeg::run(&ffmpeg_bin, &args, &cancel, |_| {}).unwrap();
    assert!(res.cancelled && !res.success);
}
