use std::path::Path;

use anyhow::{anyhow, bail, Context};

use crate::ffmpeg::Ffmpeg;
use crate::probe::{probe_file, MediaInfo};
use crate::sandbox::Sandbox;
use crate::spec::OutputSpec;

const DURATION_TOLERANCE_SECS: f64 = 1.0;

pub fn check_media(info: &MediaInfo, spec: &OutputSpec, expected_duration: f64) -> anyhow::Result<()> {
    if let Some(want) = &spec.video {
        let got = info
            .video
            .as_ref()
            .ok_or_else(|| anyhow!("output has no video stream"))?;
        if got.pix_fmt != want.pix_fmt {
            bail!("output pixel format is {} (expected {})", got.pix_fmt, want.pix_fmt);
        }
        if got.width != want.width || got.height != want.height {
            bail!(
                "output is {}x{} (expected {}x{})",
                got.width,
                got.height,
                want.width,
                want.height
            );
        }
        if got.width % 2 != 0 || got.height % 2 != 0 {
            bail!("output dimensions are not even: {}x{}", got.width, got.height);
        }
        if got.sar_num != got.sar_den {
            bail!("output sample aspect ratio is {}:{}", got.sar_num, got.sar_den);
        }
    }
    if spec.audio.is_some() && info.audio.is_none() {
        bail!("output has no audio stream");
    }

    let duration = info
        .duration
        .ok_or_else(|| anyhow!("output has no duration"))?;
    if (duration - expected_duration).abs() > DURATION_TOLERANCE_SECS {
        bail!(
            "output duration is {duration:.2}s (expected {expected_duration:.2}s)"
        );
    }
    Ok(())
}

pub fn verify_output(
    ff: &Ffmpeg,
    sb: &Sandbox,
    out: &Path,
    spec: &OutputSpec,
    expected_duration: f64,
) -> anyhow::Result<()> {
    let info = probe_file(ff, sb, out)?;
    check_media(&info, spec, expected_duration)?;

    let output = sb
        .wrap(&ff.ffmpeg)
        .args(["-hide_banner", "-nostdin", "-v", "error", "-i"])
        .arg(out)
        .args(["-f", "null", "-"])
        .output()
        .with_context(|| format!("cannot run {} for verification", ff.ffmpeg.display()))?;
    if !output.status.success() {
        bail!(
            "output does not decode: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.trim().is_empty() {
        bail!("output produced decode errors: {}", stderr.trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{AudioInfo, MediaInfo, VideoInfo};
    use crate::spec::{AudioSpec, Container, VideoCodec, VideoSpec};

    fn good_1080() -> MediaInfo {
        MediaInfo {
            duration: Some(3.0),
            video: Some(VideoInfo {
                width: 1920,
                height: 1080,
                sar_num: 1,
                sar_den: 1,
                pix_fmt: "yuv420p".into(),
                bit_depth: 8,
                interlaced: false,
                rotation: None,
                hdr: false,
                fps: Some(30.0),
            }),
            audio: Some(AudioInfo {
                channels: 2,
                sample_rate: 48000,
            }),
        }
    }

    fn spec_1080() -> OutputSpec {
        OutputSpec {
            container: Container::Mp4,
            video: Some(VideoSpec {
                codec: VideoCodec::H264,
                pix_fmt: "yuv420p",
                width: 1920,
                height: 1080,
                crf: 21,
                fps: None,
            }),
            audio: Some(AudioSpec {
                channels: 2,
                sample_rate: 48000,
                bitrate_kbps: 160,
            }),
        }
    }

    fn spec_audio_only() -> OutputSpec {
        OutputSpec {
            container: Container::M4a,
            video: None,
            audio: Some(AudioSpec {
                channels: 2,
                sample_rate: 48000,
                bitrate_kbps: 160,
            }),
        }
    }

    #[test]
    fn rejects_wrong_pix_fmt() {
        let mut info = good_1080();
        info.video.as_mut().unwrap().pix_fmt = "yuv444p".into();
        assert!(check_media(&info, &spec_1080(), 3.0).is_err());
    }

    #[test]
    fn rejects_odd_dimensions() {
        let mut info = good_1080();
        info.video.as_mut().unwrap().width = 1919;
        assert!(check_media(&info, &spec_1080(), 3.0).is_err());
    }

    #[test]
    fn rejects_duration_drift() {
        assert!(check_media(&good_1080(), &spec_1080(), 10.0).is_err());
    }

    #[test]
    fn accepts_good_output() {
        assert!(check_media(&good_1080(), &spec_1080(), 3.0).is_ok());
    }

    #[test]
    fn audio_only_requires_audio_stream() {
        let info = MediaInfo {
            duration: Some(3.0),
            video: None,
            audio: None,
        };
        assert!(check_media(&info, &spec_audio_only(), 3.0).is_err());
    }

    #[test]
    fn rejects_missing_video_stream() {
        let info = MediaInfo {
            duration: Some(3.0),
            video: None,
            audio: Some(AudioInfo {
                channels: 2,
                sample_rate: 48000,
            }),
        };
        assert!(check_media(&info, &spec_1080(), 3.0).is_err());
    }

    #[test]
    fn rejects_non_square_pixels() {
        let mut info = good_1080();
        let v = info.video.as_mut().unwrap();
        v.sar_num = 64;
        v.sar_den = 45;
        assert!(check_media(&info, &spec_1080(), 3.0).is_err());
    }
}
