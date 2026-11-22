use std::path::Path;

use crate::spec::OutputSpec;

const SIZE_MARGIN: f64 = 0.98;
const BUFSIZE_FACTOR: u64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cap {
    pub total_kbps: u32,
    pub video_kbps: u32,
}

pub fn bitrate_kbps(bytes: u64, seconds: f64) -> u32 {
    if !seconds.is_finite() || seconds <= 0.0 {
        return 0;
    }
    (bytes as f64 * 8.0 / seconds / 1000.0).round() as u32
}

pub fn plan_cap(input_bytes: u64, seconds: f64, audio_kbps: Option<u32>) -> Option<Cap> {
    if input_bytes == 0 || !seconds.is_finite() || seconds <= 0.0 {
        return None;
    }
    let allowed = input_bytes as f64 * 8.0 / seconds / 1000.0 * SIZE_MARGIN;
    if allowed < 1.0 {
        return None;
    }
    let total_kbps = allowed.floor().min(u32::MAX as f64) as u32;
    let video_allowance = match audio_kbps {
        Some(audio) => total_kbps.checked_sub(audio)?,
        None => total_kbps,
    };
    if video_allowance == 0 {
        return None;
    }
    let buffer_seconds = BUFSIZE_FACTOR as f64;
    let video_kbps =
        (f64::from(video_allowance) * seconds / (seconds + buffer_seconds)).floor() as u32;
    if video_kbps == 0 {
        return None;
    }
    Some(Cap {
        total_kbps,
        video_kbps,
    })
}

pub fn bufsize_kbps(max_kbps: u32) -> u32 {
    (u64::from(max_kbps) * BUFSIZE_FACTOR).min(u64::from(u32::MAX)) as u32
}

pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 999.95 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

pub struct Summary<'a> {
    pub output: &'a Path,
    pub output_bytes: u64,
    pub input_bytes: u64,
    pub duration: f64,
    pub spec: &'a OutputSpec,
    pub cap: Option<Cap>,
}

impl Summary<'_> {
    pub fn lines(&self) -> Vec<String> {
        let output_kbps = bitrate_kbps(self.output_bytes, self.duration);
        let input_kbps = bitrate_kbps(self.input_bytes, self.duration);
        let quality = match &self.spec.video {
            Some(video) => format!("{} CRF {}", video.codec.label(), video.crf),
            None => "audio only".to_string(),
        };
        let cap = self
            .cap
            .map(|cap| format!(", video maxrate {} kb/s", cap.video_kbps))
            .unwrap_or_default();
        let grown = self.output_bytes > self.input_bytes;

        let output_line = if grown {
            format!("output: {} ({quality}{cap})", self.output.display())
        } else {
            format!(
                "output: {} ({}, {quality}, {output_kbps} kb/s{cap})",
                self.output.display(),
                format_bytes(self.output_bytes)
            )
        };
        let input_line = if grown {
            format!(
                "warning: the output is larger than the input ({}, {input_kbps} kb/s -> {}, {output_kbps} kb/s)",
                format_bytes(self.input_bytes),
                format_bytes(self.output_bytes)
            )
        } else {
            format!(
                "input: {} total, {input_kbps} kb/s",
                format_bytes(self.input_bytes)
            )
        };

        vec![output_line, input_line]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{AudioSpec, Container, VideoCodec, VideoSpec};

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

    #[test]
    fn bitrate_uses_bytes_and_duration() {
        assert_eq!(bitrate_kbps(1_200_000_000, 3600.0), 2667);
        assert_eq!(bitrate_kbps(0, 3600.0), 0);
        assert_eq!(bitrate_kbps(1_200_000_000, 0.0), 0);
    }

    #[test]
    fn cap_subtracts_audio_from_the_input_allowance() {
        let cap = plan_cap(1_200_000_000, 3600.0, Some(160)).unwrap();
        assert_eq!(cap.total_kbps, 2613);
        assert_eq!(cap.video_kbps, 2451);
    }

    #[test]
    fn cap_uses_the_whole_allowance_without_audio() {
        let cap = plan_cap(1_200_000_000, 3600.0, None).unwrap();
        assert_eq!(cap.video_kbps, 2611);
    }

    #[test]
    fn short_clips_reserve_the_vbv_buffer() {
        let cap = plan_cap(2_515_188, 6.0, Some(64)).unwrap();
        assert_eq!(cap.total_kbps, 3286);
        assert_eq!(cap.video_kbps, 2416);
    }

    #[test]
    fn cap_is_none_when_audio_alone_exceeds_the_allowance() {
        assert_eq!(plan_cap(100_000, 60.0, Some(160)), None);
    }

    #[test]
    fn cap_ignores_empty_or_timeless_inputs() {
        assert_eq!(plan_cap(0, 3600.0, None), None);
        assert_eq!(plan_cap(1_000_000, 0.0, None), None);
        assert_eq!(plan_cap(1_000_000, f64::NAN, None), None);
    }

    #[test]
    fn bufsize_is_twice_the_max_rate() {
        assert_eq!(bufsize_kbps(2600), 5200);
        assert_eq!(bufsize_kbps(u32::MAX), u32::MAX);
    }

    #[test]
    fn sizes_are_human_readable() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(1000), "1.0 KB");
        assert_eq!(format_bytes(1_200_000_000), "1.2 GB");
        assert_eq!(format_bytes(999_950), "1.0 MB");
    }

    #[test]
    fn summary_shows_output_input_and_cap() {
        let path = Path::new("/tmp/movie-1234abcd.mp4");
        let spec = spec_1080();
        let summary = Summary {
            output: path,
            output_bytes: 1_100_000_000,
            input_bytes: 1_200_000_000,
            duration: 3600.0,
            spec: &spec,
            cap: Some(Cap {
                total_kbps: 2613,
                video_kbps: 2451,
            }),
        };

        let lines = summary.lines();
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0],
            "output: /tmp/movie-1234abcd.mp4 (1.1 GB, H.264 CRF 21, 2444 kb/s, video maxrate 2451 kb/s)"
        );
        assert_eq!(lines[1], "input: 1.2 GB total, 2667 kb/s");
    }

    #[test]
    fn growth_warning_replaces_the_input_line() {
        let path = Path::new("/tmp/movie-1234abcd.mp4");
        let spec = spec_1080();
        let summary = Summary {
            output: path,
            output_bytes: 1_500_000_000,
            input_bytes: 1_200_000_000,
            duration: 3600.0,
            spec: &spec,
            cap: None,
        };

        let lines = summary.lines();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "output: /tmp/movie-1234abcd.mp4 (H.264 CRF 21)");
        assert_eq!(
            lines[1],
            "warning: the output is larger than the input (1.2 GB, 2667 kb/s -> 1.5 GB, 3333 kb/s)"
        );
    }

    #[test]
    fn audio_only_summary_has_no_codec_details() {
        let path = Path::new("/tmp/tone-abcd1234.m4a");
        let spec = OutputSpec {
            container: Container::M4a,
            video: None,
            audio: Some(AudioSpec {
                channels: 2,
                sample_rate: 48000,
                bitrate_kbps: 160,
            }),
        };
        let summary = Summary {
            output: path,
            output_bytes: 500_000,
            input_bytes: 600_000,
            duration: 25.6,
            spec: &spec,
            cap: None,
        };

        assert_eq!(
            summary.lines()[0],
            "output: /tmp/tone-abcd1234.m4a (500.0 KB, audio only, 156 kb/s)"
        );
    }
}
