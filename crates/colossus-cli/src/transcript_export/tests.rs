use super::*;

#[test]
fn cancellation_stops_export_before_publication() {
    let control = colossus_ports::RunControl::default();
    assert!(check_cancelled(&control).is_ok());
    control.cancel();
    assert!(check_cancelled(&control).is_err());
}

#[test]
fn interactive_paths_preserve_windows_backslashes_and_explicit_formats() {
    for source in [
        r"C:\Users\Alice\conversation.md",
        r#""C:\Users\Alice Smith\conversation.md""#,
    ] {
        let (path, format) = interactive_export_arguments(source).unwrap();
        assert!(path.to_string_lossy().contains(r"C:\Users\"));
        assert!(matches!(format, TranscriptFormatArg::Markdown));
    }
    let (path, format) =
        interactive_export_arguments("'conversation 東京.txt' --format text").unwrap();
    assert_eq!(path, Path::new("conversation 東京.txt"));
    assert!(matches!(format, TranscriptFormatArg::Text));
    assert!(interactive_export_arguments("--format json transcript.json").is_ok());
    for source in [
        "",
        "-",
        "file --format yaml",
        "file --format",
        "one two",
        "'unfinished",
        "file --format json --format text",
    ] {
        assert!(interactive_export_arguments(source).is_err(), "{source}");
    }
    assert_eq!(interactive_export_line("/export file.md"), Some(" file.md"));
    assert_eq!(
        interactive_export_line("/session export file.md"),
        Some(" file.md")
    );
    assert_eq!(interactive_export_line("/exported file.md"), None);
}

#[test]
fn staged_output_never_clobbers_or_publishes_before_success() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("conversation.md");
    {
        let mut destination = Destination::new(&output).unwrap();
        destination
            .writer()
            .write_all(b"incomplete source")
            .unwrap();
        assert!(!output.exists());
    }
    assert!(!output.exists());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    let mut destination = Destination::new(&output).unwrap();
    destination
        .writer()
        .write_all(b"completed transcript")
        .unwrap();
    std::fs::write(&output, b"existing destination").unwrap();
    assert!(destination.publish().is_err());
    assert_eq!(std::fs::read(&output).unwrap(), b"existing destination");
    assert!(Destination::new(&output).is_err());
    assert!(Destination::new(&directory.path().join("missing").join("file.md")).is_err());
}

#[test]
fn successful_output_is_published_exactly_once() {
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("conversation.md");
    let mut destination = Destination::new(&output).unwrap();
    destination
        .writer()
        .write_all("東京 completed\n".as_bytes())
        .unwrap();
    destination.publish().unwrap();
    assert_eq!(std::fs::read_to_string(output).unwrap(), "東京 completed\n");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}
