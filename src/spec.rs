use anyhow::{bail, Result};

use crate::probe::{MediaInfo, VideoInfo};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    Hevc,
}

impl VideoCodec {
    pub fn label(self) -> &'static str {
        match self {
            VideoCodec::H264 => "H.264",
            VideoCodec::Hevc => "HEVC",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Mp4,
    M4a,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoSpec {
    pub codec: VideoCodec,
    pub pix_fmt: &'static str,
    pub width: u32,
    pub height: u32,
    pub crf: u32,
    pub fps: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioSpec {
    pub channels: u32,
    pub sample_rate: u32,
    pub bitrate_kbps: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputSpec {
    pub container: Container,
    pub video: Option<VideoSpec>,
    pub audio: Option<AudioSpec>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub spec: OutputSpec,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Encoders {
    pub libx264: bool,
    pub libx265: bool,
}

pub fn decide(infos: &[MediaInfo], enc: &Encoders) -> Result<Decision> {
    if infos.is_empty() {
        bail!("no media information to decide from");
    }

    let videos: Vec<&VideoInfo> = infos.iter().filter_map(|i| i.video.as_ref()).collect();

    if videos.is_empty() {
        let (audio, warnings) = audio_spec(infos);
        return Ok(Decision {
            spec: OutputSpec {
                container: Container::M4a,
                video: None,
                audio,
            },
            warnings,
        });
    }
    if videos.len() != infos.len() {
        bail!("some parts have no video stream");
    }

    if videos.iter().any(|v| v.hdr != videos[0].hdr) {
        bail!("parts mix HDR and SDR; run with --no-group to convert them separately");
    }
    let depth = videos.iter().map(|v| v.bit_depth).max().unwrap_or(8);
    if videos.iter().any(|v| v.bit_depth != depth) {
        bail!("parts mix bit depths; run with --no-group to convert them separately");
    }

    let width = even(videos.iter().map(|v| v.width).max().unwrap_or(0));
    let height = even(videos.iter().map(|v| v.height).max().unwrap_or(0));

    let mut warnings = Vec::new();
    let wants_hevc = videos[0].hdr || height > 1080;
    let codec = if wants_hevc && enc.libx265 {
        VideoCodec::Hevc
    } else if wants_hevc {
        warnings.push(
            "libx265 is not available; falling back to H.264 (the output may grow)".to_string(),
        );
        VideoCodec::H264
    } else {
        VideoCodec::H264
    };
    if codec == VideoCodec::H264 && !enc.libx264 {
        bail!("libx264 is not available; run `brew upgrade ffmpeg`");
    }

    let pix_fmt = match (codec, depth) {
        (VideoCodec::Hevc, d) if d >= 10 => "yuv420p10le",
        _ => "yuv420p",
    };
    let crf = match codec {
        VideoCodec::H264 => 21,
        VideoCodec::Hevc => 23,
    };
    let fps = if infos.len() > 1 {
        videos.iter().filter_map(|v| v.fps).reduce(f64::max)
    } else {
        None
    };

    let (audio, audio_warnings) = audio_spec(infos);
    warnings.extend(audio_warnings);

    Ok(Decision {
        spec: OutputSpec {
            container: Container::Mp4,
            video: Some(VideoSpec {
                codec,
                pix_fmt,
                width,
                height,
                crf,
                fps,
            }),
            audio,
        },
        warnings,
    })
}

fn even(v: u32) -> u32 {
    v & !1
}

fn audio_spec(infos: &[MediaInfo]) -> (Option<AudioSpec>, Vec<String>) {
    let channels: Vec<u32> = infos
        .iter()
        .filter_map(|i| i.audio.as_ref())
        .map(|a| a.channels)
        .filter(|c| *c > 0)
        .collect();
    if channels.is_empty() {
        return (None, Vec::new());
    }

    let mut warnings = Vec::new();
    let first = channels[0];
    let mut ch = if channels.iter().all(|c| *c == first) {
        first
    } else {
        warnings.push("audio channel layouts differ; mixing down to stereo".to_string());
        2
    };
    if ch > 6 {
        warnings.push("more than 6 audio channels; mixing down to 5.1".to_string());
        ch = 6;
    }
    let bitrate_kbps = match ch {
        1 => 64,
        2 => 160,
        6 => 384,
        n => (64 * n).min(512),
    };
    (
        Some(AudioSpec {
            channels: ch,
            sample_rate: 48000,
            bitrate_kbps,
        }),
        warnings,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::AudioInfo;

    fn vid(w: u32, h: u32, hdr: bool, depth: u8) -> MediaInfo {
        MediaInfo {
            duration: Some(3.0),
            video: Some(VideoInfo {
                width: w,
                height: h,
                sar_num: 1,
                sar_den: 1,
                pix_fmt: if depth == 10 {
                    "yuv420p10le"
                } else {
                    "yuv420p"
                }
                .into(),
                bit_depth: depth,
                interlaced: false,
                rotation: None,
                hdr,
                fps: Some(30.0),
            }),
            audio: Some(AudioInfo {
                channels: 2,
                sample_rate: 48000,
            }),
        }
    }

    fn enc_with_x265() -> Encoders {
        Encoders {
            libx264: true,
            libx265: true,
        }
    }

    fn enc_without_x265() -> Encoders {
        Encoders {
            libx264: true,
            libx265: false,
        }
    }

    #[test]
    fn hd_sdr_uses_h264() {
        let d = decide(&[vid(1920, 1080, false, 8)], &enc_with_x265()).unwrap();
        assert_eq!(d.spec.video.unwrap().codec, VideoCodec::H264);
    }

    #[test]
    fn uhd_uses_hevc() {
        let d = decide(&[vid(3840, 2160, false, 8)], &enc_with_x265()).unwrap();
        assert_eq!(d.spec.video.unwrap().codec, VideoCodec::Hevc);
    }

    #[test]
    fn hdr_uses_hevc_even_at_1080() {
        let d = decide(&[vid(1920, 1080, true, 10)], &enc_with_x265()).unwrap();
        let v = d.spec.video.unwrap();
        assert_eq!(v.codec, VideoCodec::Hevc);
        assert_eq!(v.pix_fmt, "yuv420p10le");
    }

    #[test]
    fn without_x265_falls_back_with_warning() {
        let d = decide(&[vid(3840, 2160, false, 8)], &enc_without_x265()).unwrap();
        assert_eq!(d.spec.video.unwrap().codec, VideoCodec::H264);
        assert_eq!(d.warnings.len(), 1);
    }

    #[test]
    fn mixed_dynamic_range_is_an_error() {
        let infos = vec![vid(1920, 1080, true, 10), vid(1920, 1080, false, 8)];
        assert!(decide(&infos, &enc_with_x265()).is_err());
    }

    #[test]
    fn mixed_channels_collapse_to_stereo() {
        let mut a = vid(1280, 720, false, 8);
        let mut b = vid(1280, 720, false, 8);
        a.audio.as_mut().unwrap().channels = 1;
        b.audio.as_mut().unwrap().channels = 6;
        let spec = decide(&[a, b], &enc_with_x265()).unwrap().spec;
        assert_eq!(spec.audio.unwrap().channels, 2);
    }

    #[test]
    fn warns_on_channel_downmix() {
        let mut a = vid(1280, 720, false, 8);
        let mut b = vid(1280, 720, false, 8);
        a.audio.as_mut().unwrap().channels = 1;
        b.audio.as_mut().unwrap().channels = 6;
        let d = decide(&[a, b], &enc_with_x265()).unwrap();
        assert!(d.warnings.iter().any(|w| w.contains("stereo")));
    }

    #[test]
    fn warns_on_more_than_six_channels() {
        let mut m = vid(1280, 720, false, 8);
        m.audio.as_mut().unwrap().channels = 8;
        let d = decide(&[m], &enc_with_x265()).unwrap();
        assert_eq!(d.spec.audio.unwrap().channels, 6);
        assert!(d.warnings.iter().any(|w| w.contains("5.1")));
    }

    #[test]
    fn audio_only_uses_m4a() {
        let m = MediaInfo {
            duration: Some(3.0),
            video: None,
            audio: Some(AudioInfo {
                channels: 2,
                sample_rate: 44100,
            }),
        };
        let spec = decide(&[m], &enc_with_x265()).unwrap().spec;
        assert_eq!(spec.container, Container::M4a);
        assert!(spec.video.is_none());
    }

    #[test]
    fn odd_dimensions_are_rounded_down() {
        let d = decide(&[vid(1919, 1079, false, 8)], &enc_with_x265()).unwrap();
        let v = d.spec.video.unwrap();
        assert_eq!((v.width, v.height), (1918, 1078));
    }

    #[test]
    fn widest_part_wins() {
        let d = decide(
            &[vid(1920, 1080, false, 8), vid(1280, 720, false, 8)],
            &enc_with_x265(),
        )
        .unwrap();
        let v = d.spec.video.unwrap();
        assert_eq!((v.width, v.height), (1920, 1080));
    }

    #[test]
    fn silent_parts_are_ignored_for_channels() {
        let mut only_audio = vid(1280, 720, false, 8);
        only_audio.audio = Some(AudioInfo {
            channels: 6,
            sample_rate: 48000,
        });
        let silent = MediaInfo {
            duration: Some(3.0),
            video: only_audio.video.clone(),
            audio: None,
        };
        let spec = decide(&[only_audio, silent], &enc_with_x265())
            .unwrap()
            .spec;
        assert_eq!(spec.audio.unwrap().channels, 6);
    }

    #[test]
    fn no_audio_anywhere_yields_no_audio_spec() {
        let mut m = vid(1280, 720, false, 8);
        m.audio = None;
        let spec = decide(&[m], &enc_with_x265()).unwrap().spec;
        assert!(spec.audio.is_none());
    }
}
