//! Noter, a focused cross-platform text and Markdown editor.
//!
//! Noter is pre-alpha. See `README.md` and `docs/ROADMAP.md` for verified
//! behavior, current limitations, and the release plan.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod markdown_ui;
mod theme;

use std::path::PathBuf;

use app::{DocumentView, LaunchOptions, NoterApp};
use theme::AppTheme;

const HELP: &str = "Noter\n\nUsage:\n  noter [FILE]\n  noter update\n  noter [--theme system|light|dark] [--view text|markdown] [FILE]\n\nDevelopment QA:\n  noter --screenshot PATH --theme light|dark --view markdown FILE";

fn main() -> eframe::Result {
    let request = match parse_launch_request(std::env::args().skip(1)) {
        Ok(request) => request,
        Err(message) => {
            eprintln!("noter: {message}\n\n{HELP}");
            return Ok(());
        }
    };
    let LaunchRequest::Run(launch) = request else {
        match request {
            LaunchRequest::Help => println!("{HELP}"),
            LaunchRequest::Version => println!("noter {}", env!("CARGO_PKG_VERSION")),
            LaunchRequest::Run(_) => unreachable!(),
        }
        return Ok(());
    };

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

#[derive(Debug)]
enum LaunchRequest {
    Run(LaunchOptions),
    Help,
    Version,
}

fn parse_launch_request(args: impl IntoIterator<Item = String>) -> Result<LaunchRequest, String> {
    let mut args = args.into_iter().peekable();
    if args.peek().is_some_and(|argument| argument == "update") {
        args.next();
        if args.next().is_some() {
            return Err("`noter update` does not accept additional arguments".to_owned());
        }
        return Ok(LaunchRequest::Run(LaunchOptions {
            show_updates: true,
            ..LaunchOptions::default()
        }));
    }

    let mut options = LaunchOptions::default();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "-h" | "--help" => return Ok(LaunchRequest::Help),
            "-V" | "--version" => return Ok(LaunchRequest::Version),
            "--theme" => {
                let value = args
                    .next()
                    .ok_or_else(|| "`--theme` requires system, light, or dark".to_owned())?;
                options.theme = Some(parse_theme(&value)?);
            }
            "--view" => {
                let value = args
                    .next()
                    .ok_or_else(|| "`--view` requires text or markdown".to_owned())?;
                options.view = Some(parse_view(&value)?);
            }
            "--screenshot" => {
                if !cfg!(feature = "screenshot-qa") {
                    return Err(
                        "`--screenshot` requires a build with feature `screenshot-qa`".to_owned(),
                    );
                }
                let value = args
                    .next()
                    .ok_or_else(|| "`--screenshot` requires an output path".to_owned())?;
                options.screenshot_path = Some(PathBuf::from(value));
            }
            _ if argument.starts_with('-') => {
                return Err(format!("unknown option `{argument}`"));
            }
            _ if options.initial_path.is_none() => {
                options.initial_path = Some(PathBuf::from(argument));
            }
            _ => return Err("only one document path may be opened at startup".to_owned()),
        }
    }
    Ok(LaunchRequest::Run(options))
}

fn parse_theme(value: &str) -> Result<AppTheme, String> {
    AppTheme::from_storage_value(value)
        .ok_or_else(|| format!("unknown theme `{value}`; expected system, light, or dark"))
}

fn parse_view(value: &str) -> Result<DocumentView, String> {
    match value {
        "text" => Ok(DocumentView::Text),
        "markdown" => Ok(DocumentView::Markdown),
        _ => Err(format!(
            "unknown document view `{value}`; expected text or markdown"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> Result<LaunchRequest, String> {
        parse_launch_request(arguments.iter().map(ToString::to_string))
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
    fn help_and_version_exit_without_opening_a_window() {
        assert!(matches!(parse(&["--help"]), Ok(LaunchRequest::Help)));
        assert!(matches!(parse(&["--version"]), Ok(LaunchRequest::Version)));
    }
}
