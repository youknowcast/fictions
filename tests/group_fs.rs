use std::path::Path;

use fictions::group::{detect_group, parent_dir};

fn write(dir: &Path, name: &str, body: &[u8]) {
    std::fs::write(dir.join(name), body).unwrap();
}

#[test]
fn does_not_strip_year_without_base_file() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "Movie (1982).mp4", b"only");

    let g = detect_group(&dir.path().join("Movie (1982).mp4")).unwrap();

    assert_eq!(g.base, "Movie (1982)");
    assert_eq!(g.parts.len(), 1);
}

#[test]
fn collects_base_and_numbered_parts() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "foo.mp4", b"a");
    write(dir.path(), "foo (1).mp4", b"b");
    write(dir.path(), "foo (2).mp4", b"c");

    let g = detect_group(&dir.path().join("foo (1).mp4")).unwrap();

    assert_eq!(g.base, "foo");
    assert_eq!(g.parts.len(), 3);
    assert_eq!(g.parts[0].order, None);
    assert_eq!(g.parts[1].order, Some(1));
    assert_eq!(g.parts[2].order, Some(2));
}

#[test]
fn ignores_other_extensions_and_unrelated_names() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "foo.mp4", b"a");
    write(dir.path(), "foo (1).mp4", b"b");
    write(dir.path(), "foo (1).mov", b"c");
    write(dir.path(), "foo (1) extra.mp4", b"d");

    let g = detect_group(&dir.path().join("foo.mp4")).unwrap();

    assert_eq!(g.parts.len(), 2);
}

#[test]
fn drops_identical_parts() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "foo.mp4", b"same");
    write(dir.path(), "foo (1).mp4", b"same");
    write(dir.path(), "foo (2).mp4", b"other");

    let g = detect_group(&dir.path().join("foo.mp4")).unwrap();

    assert_eq!(g.parts.len(), 2);
    assert_eq!(g.dropped.len(), 1);
}

#[test]
fn hash_is_stable_across_runs() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "foo.mp4", b"a");
    write(dir.path(), "foo (1).mp4", b"b");

    let first = detect_group(&dir.path().join("foo.mp4")).unwrap().hash8;
    let second = detect_group(&dir.path().join("foo (1).mp4")).unwrap().hash8;

    assert_eq!(first, second);
    assert_eq!(first.len(), 8);
}

#[test]
fn bare_file_name_has_dot_parent() {
    use std::path::Path;
    assert_eq!(parent_dir(Path::new("good.mp4")), Path::new("."));
    assert_eq!(parent_dir(Path::new("dir/good.mp4")), Path::new("dir"));
}
