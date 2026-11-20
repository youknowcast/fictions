use anyhow::{bail, Result};

use crate::probe::MediaInfo;
use crate::spec::OutputSpec;

pub fn needs_silence(infos: &[MediaInfo]) -> Vec<usize> {
    infos
        .iter()
        .enumerate()
        .filter(|(_, info)| info.video.is_some() && info.audio.is_none())
        .map(|(i, _)| i)
        .collect()
}

pub fn build(infos: &[MediaInfo], spec: &OutputSpec) -> Result<String> {
    if infos.is_empty() {
        bail!("no inputs for filtergraph");
    }
    let has_video = spec.video.is_some();
    let has_audio = spec.audio.is_some();
    if !has_video && !has_audio {
        bail!("output spec has no streams");
    }
    let layout = layout_name(spec.audio.as_ref().map_or(2, |a| a.channels));

    let mut chain: Vec<String> = Vec::new();

    for (i, info) in infos.iter().enumerate() {
        if let Some(video) = &spec.video {
            let mut filter = format!(
                "[{i}:v]scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1",
                w = video.width,
                h = video.height
            );
            if let Some(fps) = video.fps {
                filter.push_str(&format!(",fps={fps}"));
            }
            filter.push_str(&format!(",format={}[v{i}]", video.pix_fmt));
            chain.push(filter);
        }
        if has_audio {
            match &info.audio {
                Some(_) => chain.push(format!(
                    "[{i}:a]aresample=48000,aformat=sample_fmts=fltp:channel_layouts={layout}[a{i}]"
                )),
                None => {
                    let duration = info.duration.unwrap_or(0.0);
                    chain.push(format!(
                        "anullsrc=r=48000:cl={layout},atrim=duration={duration}[a{i}]"
                    ));
                }
            }
        }
    }

    let mut join = String::new();
    for i in 0..infos.len() {
        if has_video {
            join.push_str(&format!("[v{i}]"));
        }
        if has_audio {
            join.push_str(&format!("[a{i}]"));
        }
    }
    join.push_str(&format!(
        "concat=n={}:v={}:a={}",
        infos.len(),
        u8::from(has_video),
        u8::from(has_audio)
    ));
    if has_video {
        join.push_str("[outv]");
    }
    if has_audio {
        join.push_str("[outa]");
    }
    chain.push(join);

    Ok(chain.join(";"))
}

fn layout_name(channels: u32) -> &'static str {
    match channels {
        1 => "mono",
        6 => "5.1",
        _ => "stereo",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{AudioInfo, MediaInfo, VideoInfo};
    use crate::spec::{AudioSpec, Container, VideoCodec, VideoSpec};

    fn info_v(w: u32, h: u32, with_audio: bool) -> MediaInfo {
        MediaInfo {
            duration: Some(2.0),
            video: Some(VideoInfo {
                width: w,
                height: h,
                sar_num: 1,
                sar_den: 1,
                pix_fmt: "yuv420p".into(),
                bit_depth: 8,
                interlaced: false,
                rotation: None,
                hdr: false,
                fps: Some(30.0),
            }),
            audio: with_audio.then_some(AudioInfo {
                channels: 2,
                sample_rate: 48000,
            }),
        }
    }

    fn info_no_audio() -> MediaInfo {
        info_v(1920, 1080, false)
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
                fps: Some(30.0),
            }),
            audio: Some(AudioSpec {
                channels: 2,
                sample_rate: 48000,
                bitrate_kbps: 160,
            }),
        }
    }

    #[test]
    fn single_input_still_produces_graph() {
        let g = build(&[info_v(1920, 1080, true)], &spec_1080()).unwrap();
        assert!(g.contains("concat=n=1:v=1:a=1"));
        assert!(g.contains("setsar=1"));
    }

    #[test]
    fn pads_smaller_parts_to_target() {
        let g = build(
            &[info_v(1920, 1080, true), info_v(1280, 720, true)],
            &spec_1080(),
        )
        .unwrap();
        assert!(g.contains("pad=1920:1080"));
    }

    #[test]
    fn silent_parts_are_listed() {
        let idx = needs_silence(&[info_v(1920, 1080, true), info_no_audio()]);
        assert_eq!(idx, vec![1]);
    }

    #[test]
    fn graph_labels_every_input() {
        let g = build(
            &[info_v(1920, 1080, true), info_v(1920, 1080, true)],
            &spec_1080(),
        )
        .unwrap();
        assert!(g.contains("[0:v]"));
        assert!(g.contains("[1:a]"));
        assert!(g.contains("[v0][a0][v1][a1]concat=n=2:v=1:a=1"));
    }

    #[test]
    fn missing_audio_is_filled_with_silence() {
        let g = build(
            &[info_v(1920, 1080, true), info_no_audio()],
            &spec_1080(),
        )
        .unwrap();
        assert!(g.contains("anullsrc=r=48000:cl=stereo,atrim=duration=2"));
    }

    #[test]
    fn audio_only_graph_has_no_video() {
        let spec = OutputSpec {
            container: Container::M4a,
            video: None,
            audio: Some(AudioSpec {
                channels: 2,
                sample_rate: 48000,
                bitrate_kbps: 160,
            }),
        };
        let g = build(&[info_v(0, 0, true), info_v(0, 0, true)], &spec).unwrap();
        assert!(g.contains("concat=n=2:v=0:a=1"));
        assert!(!g.contains("[0:v]"));
    }
}
