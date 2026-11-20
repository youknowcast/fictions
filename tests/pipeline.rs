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
