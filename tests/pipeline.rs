mod common;

use std::path::Path;

use fictions::pipeline::{self, Options, Outcome};

fn opts() -> Options {
    Options {
        no_sandbox: true,
        ..Default::default()
    }
}

fn files(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn converts_single_file() {
    if !common::ffmpeg_ok() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    common::make_video(dir.path(), "clip.mov", 640, 360, 2.0);

    let outcome = pipeline::run(&dir.path().join("clip.mov"), &opts()).unwrap();
    let Outcome::Written(path) = outcome else {
        panic!("expected Written, got {outcome:?}");
    };

    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("clip-"), "{name}");
    assert!(name.ends_with(".mp4"), "{name}");
    assert_eq!(name.len(), "clip-".len() + 8 + ".mp4".len(), "{name}");
    assert!(path.exists());
}

#[test]
fn second_run_skips() {
    if !common::ffmpeg_ok() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    common::make_video(dir.path(), "clip.mp4", 320, 240, 1.0);

    let first = pipeline::run(&dir.path().join("clip.mp4"), &opts()).unwrap();
    let second = pipeline::run(&dir.path().join("clip.mp4"), &opts()).unwrap();

    assert!(matches!(first, Outcome::Written(_)));
    assert!(matches!(second, Outcome::Skipped(_)));
}

#[test]
fn dry_run_writes_nothing() {
    if !common::ffmpeg_ok() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    common::make_video(dir.path(), "clip.mp4", 320, 240, 1.0);
    let before = files(dir.path());

    let outcome = pipeline::run(
        &dir.path().join("clip.mp4"),
        &Options {
            dry_run: true,
            ..opts()
        },
    )
    .unwrap();

    let Outcome::DryRun(text) = outcome else {
        panic!("expected DryRun, got {outcome:?}");
    };
    assert!(text.contains("output:"));
    assert_eq!(files(dir.path()).len(), before.len());
}

fn duration_of(path: &Path) -> f64 {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

#[test]
fn concats_numbered_parts() {
    if !common::ffmpeg_ok() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    common::make_video_with(dir.path(), "foo.mp4", 320, 240, 0.7, 440.0);
    common::make_video_with(dir.path(), "foo (1).mp4", 320, 240, 0.7, 550.0);
    common::make_video_with(dir.path(), "foo (2).mp4", 320, 240, 0.7, 660.0);

    let outcome = pipeline::run(&dir.path().join("foo (1).mp4"), &opts()).unwrap();
    let Outcome::Written(path) = outcome else {
        panic!("expected Written, got {outcome:?}");
    };

    let duration = duration_of(&path);
    assert!((duration - 2.1).abs() < 0.5, "duration was {duration}");
}

#[test]
fn identical_parts_are_collapsed() {
    if !common::ffmpeg_ok() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    common::make_video(dir.path(), "bar.mp4", 320, 240, 1.0);
    std::fs::copy(dir.path().join("bar.mp4"), dir.path().join("bar (1).mp4")).unwrap();

    let outcome = pipeline::run(&dir.path().join("bar.mp4"), &opts()).unwrap();
    let Outcome::Written(path) = outcome else {
        panic!("expected Written, got {outcome:?}");
    };

    let duration = duration_of(&path);
    assert!((duration - 1.0).abs() < 0.4, "duration was {duration}");
}

#[test]
fn audio_only_input_becomes_m4a() {
    if !common::ffmpeg_ok() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    common::make_audio(dir.path(), "tone.m4a", 1.5);

    let outcome = pipeline::run(&dir.path().join("tone.m4a"), &opts()).unwrap();
    let Outcome::Written(path) = outcome else {
        panic!("expected Written, got {outcome:?}");
    };

    assert!(path.to_string_lossy().ends_with(".m4a"));
}

#[test]
fn zero_byte_input_fails() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("empty.mp4"), b"").unwrap();
    assert!(pipeline::run(&dir.path().join("empty.mp4"), &opts()).is_err());
}

#[test]
fn unwritable_directory_fails_cleanly() {
    use std::os::unix::fs::PermissionsExt;

    if !common::ffmpeg_ok() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    common::make_video(dir.path(), "locked.mp4", 320, 240, 1.0);
    let mut perms = std::fs::metadata(dir.path()).unwrap().permissions();
    perms.set_mode(0o555);
    std::fs::set_permissions(dir.path(), perms).unwrap();

    let result = pipeline::run(&dir.path().join("locked.mp4"), &opts());

    let mut perms = std::fs::metadata(dir.path()).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(dir.path(), perms).unwrap();

    assert!(result.is_err());
}

#[test]
fn truncated_input_is_suspect() {
    if !common::ffmpeg_ok() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    common::make_video(dir.path(), "broken.mp4", 320, 240, 3.0);
    let path = dir.path().join("broken.mp4");
    let bytes = std::fs::read(&path).unwrap();
    std::fs::write(&path, &bytes[..bytes.len() * 7 / 10]).unwrap();

    let outcome = pipeline::run(&path, &opts()).unwrap();
    assert!(matches!(outcome, Outcome::Suspect(_, _)), "got {outcome:?}");
}
