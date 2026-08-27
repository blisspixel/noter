//! Noter, a focused cross-platform text and Markdown editor.
//!
//! See `README.md` and `docs/ROADMAP.md` for verified behavior, current
//! limitations, and the release plan.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod bounded_text_input;
mod crash_recovery;
mod editor_settings;
mod find_ui;
mod go_to_line_ui;
mod idle_screen;
mod keyboard_nav;
mod markdown_ui;
mod theme;

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use app::{DocumentView, LaunchOptions, NoterApp};
use noter::core::file_observation::preflight_regular_file;
use theme::AppTheme;

const HELP: &str = "Noter\n\nUsage:\n  noter [OPTIONS] [--] [FILE]\n  noter update\n\nOptions:\n  --theme system|light|dark|green|amber\n  --view text|markdown\n  -h, --help\n  -V, --version\n\nFILE must name an existing readable file; Noter never creates it for you.\n`noter update` opens the local update status window and makes no network\nrequest. Option values are case-insensitive.";
const THEME_ERROR_VALUES: &str = "system, light, dark, green, or amber";

fn main() -> eframe::Result {
    let request = match parse_launch_request(std::env::args_os().skip(1)) {
        Ok(request) => request,
        Err(message) => {
            write_line(
                std::io::stderr().lock(),
                &format!("noter: {message}\n\n{HELP}"),
            );
            std::process::exit(2);
        }
    };
    let launch = match request {
        LaunchRequest::Run(launch) => launch,
        LaunchRequest::Help => {
            write_line(std::io::stdout().lock(), HELP);
            return Ok(());
        }
        LaunchRequest::Version => {
            write_line(
                std::io::stdout().lock(),
                &format!("noter {}", env!("CARGO_PKG_VERSION")),
            );
            return Ok(());
        }
    };

    if let Some(path) = launch.initial_path.as_deref()
        && let Err(message) = check_document_argument(path)
    {
        write_line(
            std::io::stderr().lock(),
            &format!("noter: {message}\n\n{HELP}"),
        );
        std::process::exit(2);
    }

    let screenshot_qa = launch.screenshot_path.is_some();
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_inner_size(if screenshot_qa {
            [1200.0, 760.0]
        } else {
            [900.0, 700.0]
        })
        .with_min_inner_size([420.0, 300.0])
        .with_transparent(false);
    if screenshot_qa {
        viewport = viewport.with_resizable(false);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Noter",
        options,
        Box::new(move |cc| Ok(Box::new(NoterApp::new(cc, launch)))),
    )
}

fn write_line(mut writer: impl std::io::Write, message: &str) {
    let _ = writeln!(writer, "{message}");
}

#[derive(Debug)]
enum LaunchRequest {
    Run(LaunchOptions),
    Help,
    Version,
}

fn parse_launch_request(args: impl IntoIterator<Item = OsString>) -> Result<LaunchRequest, String> {
    let mut args = args.into_iter().peekable();
    if args
        .peek()
        .is_some_and(|argument| argument == OsStr::new("update"))
    {
        args.next();
        if let Some(argument) = args.next() {
            // Asking a command what it does must never be the error case.
            if argument == OsStr::new("-h") || argument == OsStr::new("--help") {
                return Ok(LaunchRequest::Help);
            }
            return Err("`noter update` does not accept additional arguments".to_owned());
        }
        return Ok(LaunchRequest::Run(LaunchOptions {
            show_updates: true,
            ..LaunchOptions::default()
        }));
    }

    let mut options = LaunchOptions::default();
    let mut options_finished = false;
    while let Some(argument) = args.next() {
        if !options_finished && argument == OsStr::new("--") {
            options_finished = true;
        } else if !options_finished
            && (argument == OsStr::new("-h") || argument == OsStr::new("--help"))
        {
            return Ok(LaunchRequest::Help);
        } else if !options_finished
            && (argument == OsStr::new("-V") || argument == OsStr::new("--version"))
        {
            return Ok(LaunchRequest::Version);
        } else if !options_finished && argument == OsStr::new("--theme") {
            let value = args
                .next()
                .ok_or_else(|| format!("`--theme` requires {THEME_ERROR_VALUES}"))?;
            let value = unicode_option_value("--theme", value)?;
            options.theme = Some(parse_theme(&value)?);
        } else if !options_finished && argument == OsStr::new("--view") {
            let value = args
                .next()
                .ok_or_else(|| "`--view` requires text or markdown".to_owned())?;
            let value = unicode_option_value("--view", value)?;
            options.view = Some(parse_view(&value)?);
        } else if !options_finished && argument == OsStr::new("--screenshot") {
            if !cfg!(feature = "screenshot-qa") {
                return Err(
                    "`--screenshot` requires a build with feature `screenshot-qa`".to_owned(),
                );
            }
            let value = args
                .next()
                .ok_or_else(|| "`--screenshot` requires an output path".to_owned())?;
            options.screenshot_path = Some(PathBuf::from(value));
        } else if !options_finished && argument == OsStr::new("--screenshot-idle") {
            if !cfg!(feature = "screenshot-qa") {
                return Err(
                    "`--screenshot-idle` requires a build with feature `screenshot-qa`".to_owned(),
                );
            }
            options.screenshot_idle = true;
        } else if !options_finished && argument.to_string_lossy().starts_with('-') {
            let Some(argument) = argument.to_str() else {
                return Err("option names must be valid Unicode".to_owned());
            };
            return Err(format!("unknown option `{}`", escaped_cli_value(argument)));
        } else if options.initial_path.is_none() {
            options.initial_path = Some(PathBuf::from(argument));
        } else {
            return Err("only one document path may be opened at startup".to_owned());
        }
    }
    if options.screenshot_idle && options.screenshot_path.is_none() {
        return Err("`--screenshot-idle` requires `--screenshot PATH`".to_owned());
    }
    Ok(LaunchRequest::Run(options))
}

/// Rejects a startup document argument that cannot name an openable file.
///
/// A mistyped path, a directory, or a file the user cannot read is an argument
/// mistake, so it fails on the command line exactly like an invalid `--theme`
/// rather than opening a window that looks like a new blank document. Problems
/// with the file's *content*, such as invalid UTF-8 or an oversized document,
/// still open the window and report there, because those also arrive from the
/// Open dialog and from a desktop file association.
fn check_document_argument(path: &Path) -> Result<(), String> {
    let cannot_open = |reason: &str| format!("cannot open `{}`: {reason}", escaped_cli_path(path));
    preflight_regular_file(path).map_err(|error| cannot_open(&open_failure(&error)))
}

/// Describes a filesystem failure without leaking the operating-system code.
fn open_failure(error: &std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::NotFound => "no such file".to_owned(),
        std::io::ErrorKind::PermissionDenied => "permission denied".to_owned(),
        _ => error.to_string(),
    }
}

fn unicode_option_value(option: &str, value: OsString) -> Result<String, String> {
    value
        .into_string()
        .map_err(|_| format!("`{option}` value must be valid Unicode"))
}

fn parse_theme(value: &str) -> Result<AppTheme, String> {
    AppTheme::from_storage_value(&value.to_ascii_lowercase()).ok_or_else(|| {
        format!(
            "unknown theme `{}`; expected {THEME_ERROR_VALUES}",
            escaped_cli_value(value)
        )
    })
}

fn parse_view(value: &str) -> Result<DocumentView, String> {
    match value.to_ascii_lowercase().as_str() {
        "text" => Ok(DocumentView::Text),
        "markdown" => Ok(DocumentView::Markdown),
        _ => Err(format!(
            "unknown document view `{}`; expected text or markdown",
            escaped_cli_value(value)
        )),
    }
}

fn escaped_cli_value(value: &str) -> String {
    value.escape_debug().collect()
}

/// Escapes only control characters so a reported path stays readable.
///
/// Full debug escaping would double every separator in a Windows path, which
/// makes the message harder to read than the mistake it reports.
fn escaped_cli_path(path: &Path) -> String {
    let mut escaped = String::new();
    for character in path.display().to_string().chars() {
        if character.is_control() {
            escaped.extend(character.escape_debug());
        } else {
            escaped.push(character);
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ClosedOutput;

    impl std::io::Write for ClosedOutput {
        fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn parse(arguments: &[&str]) -> Result<LaunchRequest, String> {
        parse_launch_request(arguments.iter().map(OsString::from))
    }

    #[test]
    fn document_theme_and_view_are_parsed_together() {
        let LaunchRequest::Run(options) =
            parse(&["--theme", "light", "--view", "markdown", "guide.md"])
                .expect("valid launch arguments should parse")
        else {
            panic!("the arguments should launch the application");
        };

        assert_eq!(options.theme, Some(AppTheme::Light));
        assert_eq!(options.view, Some(DocumentView::Markdown));
        assert_eq!(options.initial_path, Some(PathBuf::from("guide.md")));
    }

    #[test]
    fn specialty_themes_are_available_from_the_command_line() {
        for (value, expected) in [
            ("green", AppTheme::GreenScreen),
            ("amber", AppTheme::AmberScreen),
        ] {
            let LaunchRequest::Run(options) =
                parse(&["--theme", value]).expect("specialty theme should parse")
            else {
                panic!("the arguments should launch the application");
            };
            assert_eq!(options.theme, Some(expected));
        }
    }

    #[test]
    fn update_command_opens_the_in_app_update_status() {
        let LaunchRequest::Run(options) = parse(&["update"]).expect("update should parse") else {
            panic!("update should launch the application");
        };

        assert!(options.show_updates);
        assert!(options.initial_path.is_none());
    }

    #[test]
    fn invalid_values_and_extra_paths_are_rejected() {
        assert!(parse(&["--theme", "blue"]).is_err());
        assert!(parse(&["--view", "preview"]).is_err());
        assert!(parse(&["first.txt", "second.txt"]).is_err());
        assert!(parse(&["update", "note.md"]).is_err());
    }

    #[test]
    fn invalid_values_cannot_inject_terminal_control_characters() {
        for arguments in [
            ["--theme", "bad\n\u{1b}[31m"],
            ["--view", "bad\r\u{7}"],
            ["-bad\n\u{1b}[2J", ""],
        ] {
            let arguments = if arguments[1].is_empty() {
                &arguments[..1]
            } else {
                &arguments[..]
            };
            let error = parse(arguments).expect_err("control-bearing values must be rejected");

            assert!(!error.contains(['\n', '\r', '\u{1b}', '\u{7}']));
            assert!(error.contains('\\'));
        }
        assert!(
            parse(&["--theme", "grün"])
                .expect_err("an unknown Unicode theme must be rejected")
                .contains("grün")
        );
    }

    #[test]
    fn option_values_are_case_insensitive() {
        let LaunchRequest::Run(options) = parse(&["--theme", "LIGHT", "--view", "Markdown"])
            .expect("shouted option values should parse")
        else {
            panic!("the arguments should launch the application");
        };

        assert_eq!(options.theme, Some(AppTheme::Light));
        assert_eq!(options.view, Some(DocumentView::Markdown));
    }

    #[test]
    fn update_explains_itself_instead_of_rejecting_help() {
        assert!(matches!(
            parse(&["update", "--help"]),
            Ok(LaunchRequest::Help)
        ));
        assert!(matches!(parse(&["update", "-h"]), Ok(LaunchRequest::Help)));
        assert!(parse(&["update", "note.md"]).is_err());
    }

    #[test]
    fn a_readable_file_argument_is_accepted() {
        let directory = tempfile::tempdir().expect("a temporary directory should be available");
        let path = directory.path().join("notes.md");
        std::fs::write(&path, "# notes\n").expect("the fixture should be writable");

        assert_eq!(check_document_argument(&path), Ok(()));
    }

    #[test]
    fn content_validation_remains_deferred_to_the_gui_loader() {
        let directory = tempfile::tempdir().expect("a temporary directory should be available");
        let path = directory.path().join("invalid-utf8.txt");
        std::fs::write(&path, [0xff]).expect("the fixture should be writable");

        assert_eq!(check_document_argument(&path), Ok(()));
    }

    #[test]
    fn a_missing_file_argument_fails_closed_with_its_path() {
        let directory = tempfile::tempdir().expect("a temporary directory should be available");
        let path = directory.path().join("does-not-exist.md");

        let error = check_document_argument(&path).expect_err("a missing path must be refused");

        assert!(error.starts_with("cannot open `"));
        assert!(error.ends_with(": no such file"));
        assert!(error.contains("does-not-exist.md"));
    }

    #[test]
    fn a_directory_argument_fails_closed() {
        let directory = tempfile::tempdir().expect("a temporary directory should be available");

        let error =
            check_document_argument(directory.path()).expect_err("a directory must be refused");

        assert!(error.ends_with(": path is a directory"));
    }

    #[cfg(unix)]
    #[test]
    fn a_final_symlink_argument_fails_before_gui_launch() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("a temporary directory should be available");
        let target = directory.path().join("target.txt");
        let link = directory.path().join("link.txt");
        std::fs::write(&target, "target").expect("the target should be writable");
        symlink(&target, &link).expect("the symlink fixture should be creatable");

        let error = check_document_argument(&link).expect_err("a final symlink must be refused");

        assert!(error.ends_with(": path is a symbolic link or reparse point"));
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_argument_is_rejected_without_opening_it() {
        use std::process::Command;

        let directory = tempfile::tempdir().expect("a temporary directory should be available");
        let path = directory.path().join("notes.pipe");
        let status = Command::new("mkfifo")
            .arg(&path)
            .status()
            .expect("the POSIX mkfifo utility should be available");
        assert!(status.success(), "the FIFO fixture should be creatable");

        let error = check_document_argument(&path).expect_err("a FIFO must be refused");

        assert!(error.ends_with(": path is not a supported regular file"));
    }

    #[cfg(windows)]
    #[test]
    fn a_final_symlink_argument_fails_before_gui_launch() {
        use std::os::windows::fs::symlink_file;

        const ERROR_PRIVILEGE_NOT_HELD: i32 = 1_314;

        let directory = tempfile::tempdir().expect("a temporary directory should be available");
        let target = directory.path().join("target.txt");
        let link = directory.path().join("link.txt");
        std::fs::write(&target, "target").expect("the target should be writable");
        if let Err(error) = symlink_file(&target, &link) {
            if error.raw_os_error() == Some(ERROR_PRIVILEGE_NOT_HELD) {
                return;
            }
            panic!("the symlink fixture should be creatable: {error}");
        }

        let error = check_document_argument(&link).expect_err("a final symlink must be refused");

        assert!(error.ends_with(": path is a symbolic link or reparse point"));
    }

    #[test]
    fn reported_paths_cannot_inject_terminal_control_characters() {
        let escaped = escaped_cli_path(Path::new("notes\n\u{1b}[31m.md"));

        assert!(!escaped.contains(['\n', '\u{1b}']));
        assert!(escaped.contains("\\n"));
        // Ordinary separators stay readable rather than doubled.
        assert_eq!(escaped_cli_path(Path::new("a/b c.md")), "a/b c.md");
    }

    #[test]
    fn help_and_version_exit_without_opening_a_window() {
        assert!(matches!(parse(&["--help"]), Ok(LaunchRequest::Help)));
        assert!(matches!(parse(&["--version"]), Ok(LaunchRequest::Version)));
    }

    #[test]
    fn closed_cli_output_does_not_panic() {
        write_line(ClosedOutput, concat!("noter ", env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn option_terminator_allows_paths_that_start_with_a_dash() {
        let LaunchRequest::Run(options) =
            parse(&["--", "-meeting-notes.md"]).expect("the escaped path should parse")
        else {
            panic!("the escaped path should launch the application");
        };

        assert_eq!(
            options.initial_path,
            Some(PathBuf::from("-meeting-notes.md"))
        );
    }

    #[cfg(unix)]
    fn non_unicode_path() -> OsString {
        use std::os::unix::ffi::OsStringExt;

        OsString::from_vec(b"notes-\xff.md".to_vec())
    }

    #[cfg(windows)]
    fn non_unicode_path() -> OsString {
        use std::os::windows::ffi::OsStringExt;

        OsString::from_wide(&[
            u16::from(b'n'),
            u16::from(b'o'),
            u16::from(b't'),
            u16::from(b'e'),
            0xd800,
        ])
    }

    #[test]
    fn document_paths_preserve_non_unicode_operating_system_values() {
        let path = non_unicode_path();
        let LaunchRequest::Run(options) =
            parse_launch_request([path.clone()]).expect("an OS-native path should parse")
        else {
            panic!("the path should launch the application");
        };

        assert_eq!(options.initial_path, Some(PathBuf::from(path)));
    }

    #[test]
    fn non_unicode_option_values_return_controlled_errors() {
        let error = parse_launch_request([OsString::from("--theme"), non_unicode_path()])
            .expect_err("theme values must be Unicode");

        assert_eq!(error, "`--theme` value must be valid Unicode");
    }

    #[cfg(feature = "screenshot-qa")]
    #[test]
    fn idle_screenshot_is_an_explicit_test_only_capture_mode() {
        assert!(parse(&["--screenshot-idle"]).is_err());
        let LaunchRequest::Run(options) = parse(&[
            "--theme",
            "green",
            "--screenshot",
            "idle.png",
            "--screenshot-idle",
        ])
        .expect("complete idle screenshot arguments should parse") else {
            panic!("the capture arguments should launch the application");
        };

        assert_eq!(options.theme, Some(AppTheme::GreenScreen));
        assert_eq!(options.screenshot_path, Some(PathBuf::from("idle.png")));
        assert!(options.screenshot_idle);
    }
}
