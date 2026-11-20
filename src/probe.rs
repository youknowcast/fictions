use anyhow::{anyhow, Context};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct MediaInfo {
    pub duration: Option<f64>,
    pub video: Option<VideoInfo>,
    pub audio: Option<AudioInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub sar_num: u32,
    pub sar_den: u32,
    pub pix_fmt: String,
    pub bit_depth: u8,
    pub interlaced: bool,
    pub rotation: Option<f64>,
    pub hdr: bool,
    pub fps: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioInfo {
    pub channels: u32,
    pub sample_rate: u32,
}

pub fn pix_fmt_depth(pix_fmt: &str) -> u8 {
    for depth in [12u8, 10u8] {
        if pix_fmt.contains(&depth.to_string()) {
            return depth;
        }
    }
    8
}

pub fn parse_probe_json(json: &str) -> anyhow::Result<MediaInfo> {
    let root: Value = serde_json::from_str(json).context("ffprobe output is not JSON")?;
    let streams = root["streams"]
        .as_array()
        .ok_or_else(|| anyhow!("ffprobe output has no streams"))?;

    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video" && s["disposition"]["attached_pic"] != 1)
        .map(parse_video)
        .transpose()?;

    let audio = streams
        .iter()
        .find(|s| s["codec_type"] == "audio")
        .map(parse_audio)
        .transpose()?;

    let duration = root["format"]["duration"]
        .as_str()
        .and_then(|d| d.parse::<f64>().ok())
        .or_else(|| {
            streams
                .iter()
                .filter_map(|s| s["duration"].as_str())
                .filter_map(|d| d.parse::<f64>().ok())
                .reduce(f64::max)
        });

    Ok(MediaInfo {
        duration,
        video,
        audio,
    })
}

fn parse_video(stream: &Value) -> anyhow::Result<VideoInfo> {
    let width = stream["width"]
        .as_u64()
        .ok_or_else(|| anyhow!("video stream has no width"))? as u32;
    let height = stream["height"]
        .as_u64()
        .ok_or_else(|| anyhow!("video stream has no height"))? as u32;
    let pix_fmt = stream["pix_fmt"].as_str().unwrap_or("unknown").to_string();

    let (sar_num, sar_den) = match stream["sample_aspect_ratio"].as_str() {
        Some(spec) => {
            let (n, d) = spec
                .split_once(':')
                .and_then(|(n, d)| Some((n.parse::<u32>().ok()?, d.parse::<u32>().ok()?)))
                .unwrap_or((1, 1));
            if n == 0 || d == 0 {
                (1, 1)
            } else {
                (n, d)
            }
        }
        None => (1, 1),
    };

    let bit_depth = stream["bits_per_raw_sample"]
        .as_str()
        .and_then(|b| b.parse::<u8>().ok())
        .unwrap_or_else(|| pix_fmt_depth(&pix_fmt));

    let interlaced = stream["field_order"]
        .as_str()
        .is_some_and(|f| f != "progressive");

    let rotation = stream["side_data_list"]
        .as_array()
        .and_then(|list| list.iter().find_map(|sd| sd["rotation"].as_f64()));

    let transfer = stream["color_transfer"].as_str().unwrap_or("");
    let primaries = stream["color_primaries"].as_str().unwrap_or("");
    let hdr = matches!(transfer, "smpte2084" | "arib-std-b67") || primaries == "bt2020";

    let fps = stream["avg_frame_rate"]
        .as_str()
        .or_else(|| stream["r_frame_rate"].as_str())
        .and_then(parse_rate);

    Ok(VideoInfo {
        width,
        height,
        sar_num,
        sar_den,
        pix_fmt,
        bit_depth,
        interlaced,
        rotation,
        hdr,
        fps,
    })
}

fn parse_rate(spec: &str) -> Option<f64> {
    let (n, d) = spec.split_once('/')?;
    let n: f64 = n.parse().ok()?;
    let d: f64 = d.parse().ok()?;
    if n <= 0.0 || d <= 0.0 {
        return None;
    }
    Some(n / d)
}

fn parse_audio(stream: &Value) -> anyhow::Result<AudioInfo> {
    Ok(AudioInfo {
        channels: stream["channels"].as_u64().unwrap_or(0) as u32,
        sample_rate: stream["sample_rate"]
            .as_str()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_video_and_audio() {
        let json = include_str!("../tests/fixtures/probe/basic.json");
        let m = parse_probe_json(json).unwrap();
        let v = m.video.unwrap();
        assert_eq!((v.width, v.height), (1920, 1080));
        assert_eq!((v.sar_num, v.sar_den), (1, 1));
        assert!(!v.hdr);
        assert_eq!(v.fps, Some(30.0));
        assert_eq!(m.audio.unwrap().channels, 2);
    }

    #[test]
    fn detects_hdr_and_10bit() {
        let json = include_str!("../tests/fixtures/probe/hdr.json");
        let m = parse_probe_json(json).unwrap();
        let v = m.video.unwrap();
        assert!(v.hdr);
        assert_eq!(v.bit_depth, 10);
        assert_eq!(v.rotation, Some(-90.0));
    }

    #[test]
    fn audio_only_has_no_video() {
        let json = include_str!("../tests/fixtures/probe/audio.json");
        assert!(parse_probe_json(json).unwrap().video.is_none());
    }

    #[test]
    fn depth_from_pix_fmt() {
        assert_eq!(pix_fmt_depth("yuv420p"), 8);
        assert_eq!(pix_fmt_depth("yuv420p10le"), 10);
        assert_eq!(pix_fmt_depth("yuv420p12le"), 12);
    }

    #[test]
    fn zero_byte_json_fails() {
        assert!(parse_probe_json("").is_err());
    }
}
