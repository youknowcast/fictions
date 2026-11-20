use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{bail, Context};

use crate::ffmpeg::Ffmpeg;
use crate::sandbox::Sandbox;
use crate::spec::{OutputSpec, VideoCodec};

pub struct Request<'a> {
    pub inputs: &'a [PathBuf],
    pub spec: &'a OutputSpec,
    pub output: &'a Path,
    pub progress: bool,
    pub interlaced: bool,
    pub duration: Option<f64>,
    pub filtergraph: Option<&'a str>,
}

pub struct EncodeOutcome {
    pub stderr: String,
}

pub fn build_args(req: &Request) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-hide_banner".into(),
        "-nostdin".into(),
        "-y".into(),
        "-v".into(),
        "error".into(),
        "-nostats".into(),
    ];
    if req.progress {
        args.push("-progress".into());
        args.push("pipe:1".into());
    }
    args.push("-protocol_whitelist".into());
    args.push("file".into());
    for input in req.inputs {
        args.push("-i".into());
        args.push(input.to_string_lossy().into_owned());
    }

    match req.filtergraph {
        Some(graph) => {
            args.push("-filter_complex".into());
            args.push(graph.to_string());
            if req.spec.video.is_some() {
                args.push("-map".into());
                args.push("[outv]".into());
            }
            if req.spec.audio.is_some() {
                args.push("-map".into());
                args.push("[outa]".into());
            }
        }
        None => {
            if req.spec.video.is_some() {
                args.push("-map".into());
                args.push("0:V:0?".into());
            }
            if req.spec.audio.is_some() {
                args.push("-map".into());
                args.push("0:a:0?".into());
            }
        }
    }
    args.push("-map_metadata".into());
    args.push("-1".into());
    args.push("-map_chapters".into());
    args.push("-1".into());
    args.push("-dn".into());
    args.push("-sn".into());

    if let Some(video) = &req.spec.video {
        if req.filtergraph.is_none() {
            let mut filter = "scale=trunc(iw/2)*2:trunc(ih/2)*2,setsar=1".to_string();
            if req.interlaced {
                filter.push_str(",yadif");
            }
            args.push("-vf".into());
            args.push(filter);
        }

        let encoder = match video.codec {
            VideoCodec::H264 => "libx264",
            VideoCodec::Hevc => "libx265",
        };
        args.push("-c:v".into());
        args.push(encoder.into());
        args.push("-crf".into());
        args.push(video.crf.to_string());
        args.push("-preset".into());
        args.push("medium".into());
        if video.codec == VideoCodec::H264 {
            args.push("-profile:v".into());
            args.push("high".into());
        } else {
            args.push("-tag:v".into());
            args.push("hvc1".into());
        }
        args.push("-pix_fmt".into());
        args.push(video.pix_fmt.into());
    }

    if let Some(audio) = &req.spec.audio {
        args.push("-c:a".into());
        args.push("aac".into());
        args.push("-ar".into());
        args.push(audio.sample_rate.to_string());
        args.push("-ac".into());
        args.push(audio.channels.to_string());
        args.push("-b:a".into());
        args.push(format!("{}k", audio.bitrate_kbps));
    }

    args.push("-movflags".into());
    args.push("+faststart".into());
    args.push(req.output.to_string_lossy().into_owned());
    args
}

pub fn run(ff: &Ffmpeg, sb: &Sandbox, req: &Request) -> anyhow::Result<EncodeOutcome> {
    let args = build_args(req);
    let mut cmd = sb.wrap(&ff.ffmpeg);
    cmd.args(&args)
        .stdin(Stdio::null())
        .stdout(if req.progress {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .with_context(|| format!("cannot run {}", ff.ffmpeg.display()))?;

    if let Some(stdout) = child.stdout.take() {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(us) = line.strip_prefix("out_time_us=") {
                if let Ok(us) = us.trim().parse::<i64>() {
                    if let Some(pct) = percent_from(req, us) {
                        eprint!("\r{pct:>5.1}%");
                    }
                }
            }
        }
        if req.progress {
            eprintln!();
        }
    }

    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        pipe.read_to_string(&mut stderr)?;
    }
    let status = child.wait()?;
    if !status.success() {
        bail!("ffmpeg failed: {}", stderr.trim());
    }
    Ok(EncodeOutcome { stderr })
}

pub fn percent_from(req: &Request, out_time_us: i64) -> Option<f64> {
    let total = req.duration?;
    if total <= 0.0 {
        return None;
    }
    Some((out_time_us as f64 / 1_000_000.0 / total * 100.0).clamp(0.0, 100.0))
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

    fn spec_hevc() -> OutputSpec {
        let mut spec = spec_1080();
        spec.video = Some(VideoSpec {
            codec: VideoCodec::Hevc,
            pix_fmt: "yuv420p10le",
            width: 3840,
            height: 2160,
            crf: 23,
            fps: None,
        });
        spec
    }

    fn req_single<'a>(spec: &'a OutputSpec, inputs: &'a [PathBuf], output: &'a Path) -> Request<'a> {
        Request {
            inputs,
            spec,
            output,
            progress: false,
            interlaced: false,
            duration: None,
            filtergraph: None,
        }
    }

    fn args_for(spec: &OutputSpec) -> Vec<String> {
        let inputs = vec![PathBuf::from("in.mp4")];
        let output = PathBuf::from("out.mp4");
        build_args(&req_single(spec, &inputs, &output))
    }

    #[test]
    fn maps_only_first_real_video_and_audio() {
        let args = args_for(&spec_1080());
        assert!(args.windows(2).any(|w| w[0] == "-map" && w[1] == "0:V:0?"));
        assert!(args.windows(2).any(|w| w[0] == "-map" && w[1] == "0:a:0?"));
    }

    #[test]
    fn strips_metadata_and_attachments() {
        let args = args_for(&spec_1080());
        assert!(args.windows(2).any(|w| w[0] == "-map_metadata" && w[1] == "-1"));
        assert!(args.contains(&"-map_chapters".to_string()));
        assert!(args.contains(&"-sn".to_string()));
        assert!(args.contains(&"-dn".to_string()));
    }

    #[test]
    fn h264_settings() {
        let args = args_for(&spec_1080());
        assert!(args.windows(2).any(|w| w[0] == "-c:v" && w[1] == "libx264"));
        assert!(args.windows(2).any(|w| w[0] == "-crf" && w[1] == "21"));
        assert!(args.windows(2).any(|w| w[0] == "-pix_fmt" && w[1] == "yuv420p"));
        assert!(args.windows(2).any(|w| w[0] == "-movflags" && w[1] == "+faststart"));
    }

    #[test]
    fn hevc_is_tagged_hvc1() {
        let args = args_for(&spec_hevc());
        assert!(args.windows(2).any(|w| w[0] == "-tag:v" && w[1] == "hvc1"));
        assert!(args.windows(2).any(|w| w[0] == "-crf" && w[1] == "23"));
    }

    #[test]
    fn audio_is_aac_48k() {
        let args = args_for(&spec_1080());
        assert!(args.windows(2).any(|w| w[0] == "-c:a" && w[1] == "aac"));
        assert!(args.windows(2).any(|w| w[0] == "-ar" && w[1] == "48000"));
        assert!(args.windows(2).any(|w| w[0] == "-ac" && w[1] == "2"));
        assert!(args.windows(2).any(|w| w[0] == "-b:a" && w[1] == "160k"));
    }

    #[test]
    fn scale_only_rounds_even_and_sets_sar() {
        let args = args_for(&spec_1080());
        let vf = args.windows(2).find(|w| w[0] == "-vf").unwrap()[1].clone();
        assert!(vf.contains("scale=trunc(iw/2)*2:trunc(ih/2)*2"));
        assert!(vf.contains("setsar=1"));
        assert!(!vf.contains("yadif"));
    }

    #[test]
    fn interlaced_adds_yadif() {
        let inputs = vec![PathBuf::from("in.mp4")];
        let output = PathBuf::from("out.mp4");
        let spec = spec_1080();
        let mut req = req_single(&spec, &inputs, &output);
        req.interlaced = true;
        let vf = build_args(&req)
            .windows(2)
            .find(|w| w[0] == "-vf")
            .unwrap()[1]
            .clone();
        assert!(vf.contains("yadif"));
    }

    #[test]
    fn audio_only_has_no_video_args() {
        let inputs = vec![PathBuf::from("in.m4a")];
        let output = PathBuf::from("out.m4a");
        let spec = OutputSpec {
            container: Container::M4a,
            video: None,
            audio: Some(AudioSpec {
                channels: 2,
                sample_rate: 48000,
                bitrate_kbps: 160,
            }),
        };
        let args = build_args(&req_single(&spec, &inputs, &output));
        assert!(args.windows(2).any(|w| w[0] == "-map" && w[1] == "0:a:0?"));
        assert!(!args.contains(&"-c:v".to_string()));
        assert!(!args.contains(&"-vf".to_string()));
    }

    #[test]
    fn progress_flag_adds_progress_pipe() {
        let inputs = vec![PathBuf::from("in.mp4")];
        let output = PathBuf::from("out.mp4");
        let spec = spec_1080();
        let mut req = req_single(&spec, &inputs, &output);
        req.progress = true;
        let args = build_args(&req);
        assert!(args.windows(2).any(|w| w[0] == "-progress" && w[1] == "pipe:1"));
    }

    #[test]
    fn filtergraph_mode_maps_graph_outputs() {
        let inputs = vec![PathBuf::from("a.mp4"), PathBuf::from("b.mp4")];
        let output = PathBuf::from("out.mp4");
        let spec = spec_1080();
        let mut req = req_single(&spec, &inputs, &output);
        req.filtergraph = Some("[0:v]null[outv];[0:a]anull[outa]");

        let args = build_args(&req);

        assert!(args.windows(2).any(|w| w[0] == "-filter_complex"));
        assert!(args.windows(2).any(|w| w[0] == "-map" && w[1] == "[outv]"));
        assert!(args.windows(2).any(|w| w[0] == "-map" && w[1] == "[outa]"));
        assert!(!args.contains(&"-vf".to_string()));
    }

    #[test]
    fn total_duration_comes_from_duration() {
        let inputs = vec![PathBuf::from("in.mp4")];
        let output = PathBuf::from("out.mp4");
        let spec = spec_1080();
        let mut req = req_single(&spec, &inputs, &output);
        req.duration = Some(12.0);
        assert_eq!(percent_from(&req, 6_000_000), Some(50.0));
    }
}
