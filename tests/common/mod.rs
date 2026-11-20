use std::path::Path;
use std::process::Command;

pub fn ffmpeg_ok() -> bool {
    Command::new("ffmpeg")
        .args(["-hide_banner", "-version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn make_video(dir: &Path, name: &str, w: u32, h: u32, secs: f32) {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("testsrc2=size={w}x{h}:rate=30:duration={secs}"))
        .args(["-f", "lavfi", "-i"])
        .arg(format!("sine=frequency=440:duration={secs}"))
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(dir.join(name))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "failed to generate {name}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn make_audio(dir: &Path, name: &str, secs: f32) {
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("sine=frequency=440:duration={secs}"))
        .args(["-c:a", "aac"])
        .arg(dir.join(name))
        .status()
        .unwrap();
    assert!(status.success(), "failed to generate {name}");
}
