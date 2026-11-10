use std::env::{var, var_os};
use std::fs::read_to_string;
use std::io::IsTerminal;
use std::panic::Location;
use std::result::Result as StdResult;
use std::{env::current_dir, error::Error as StdError};
use std::{fmt, io};
use tree_sitter_highlight::{Highlight, HighlightConfiguration, HighlightEvent, Highlighter};
use tree_sitter_rust::{HIGHLIGHTS_QUERY, INJECTIONS_QUERY, LANGUAGE};

/// How many lines above and below a thrown `Error` that are shown to the user in the output
const CONTEXT_LINES: usize = 2;
/// The ENV var that lets a user override the default
const CONTEXT_LINES_VAR: &str = "DUCTILE_SYSTEMS_CONTEXT_LINES";

// Colors for highlighting code when an error is thrown
const RESET: &str = "\x1b[0m";
const BOLD_RED: &str = "\x1b[1;31m";
const BOLD_BLUE: &str = "\x1b[1;34m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const BLUE: &str = "\x1b[34m";
const MAGENTA: &str = "\x1b[35m";
const CYAN: &str = "\x1b[36m";
const GREY: &str = "\x1b[90m";
const PLAIN: &str = "";

/// All of the tree sitter grammar types and the associated color for outputting to the terminal
const PALETTE: &[(&str, &str)] = &[
    ("attribute", CYAN),
    ("comment", GREY),
    ("constant", CYAN),
    ("constructor", YELLOW),
    ("escape", CYAN),
    ("function", BLUE),
    ("keyword", MAGENTA),
    ("label", YELLOW),
    ("operator", PLAIN),
    ("property", PLAIN),
    ("punctuation", PLAIN),
    ("string", GREEN),
    ("type", YELLOW),
    ("variable.builtin", MAGENTA),
    ("variable.parameter", PLAIN),
];

/// `Error` is a type meant to be used in the return type for Rust tests. This type brings a few
/// advantages over a regular error type:
///
/// 1. It works with any error that implements `std::error::Error` much like `anyhow::Error`
/// 2. It is purpose built for tests and prints out a helpful error message to see where the error
///    ocurred and the surrounding code context
/// 3. The line number and file it ocurred in
/// 4. Syntax colored output for failed tests
///
/// Here is an example of what it looks like when a test fails:
///
/// ```bash
/// ---- tests::demo_missing_file stdout ----
/// Error: No such file or directory (os error 2)
///   --> testing/src/lib.rs:276:24
///     |
/// 274 |     #[ignore = "fails on purpose to show the output"]
/// 275 |     fn demo_missing_file() -> Result<(), Error> {
/// 276 |         let contents = fs::read_to_string("/definitely/not/here")?;
///     |                        ^
/// 277 |         assert!(contents.is_empty());
/// 278 |         Ok(())
/// ```
///
/// You can see exactly what line failed, what the error was, and where. Perfect for someone trying
/// to nail down a test failure.
///
/// If you would like more or less lines you can set the env var `DUCTILE_SYSTEMS_CONTEXT_LINES` to
/// a number greater than or equal to 0.
pub struct Error {
    source: Box<dyn StdError + Send + Sync>,
    location: &'static Location<'static>,
}

pub type Result<T = (), E = Error> = StdResult<T, E>;

impl<E: Into<Box<dyn StdError + Send + Sync>>> From<E> for Error {
    #[track_caller]
    fn from(source: E) -> Self {
        Self {
            source: source.into(),
            location: Location::caller(),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.render(f, use_color(), context_lines())
    }
}

impl Error {
    fn render(&self, f: &mut dyn fmt::Write, color: bool, context: usize) -> fmt::Result {
        // Get our file, line, and column values from #[track_caller]
        let (file, line, column) = (
            self.location.file(),
            self.location.line() as usize,
            self.location.column() as usize,
        );

        // Get what colors we need for the non rust code parts of our rendered output
        // such as the error string
        let (red, blue, reset) = if color {
            (BOLD_RED, BOLD_BLUE, RESET)
        } else {
            ("", "", "")
        };

        // Write out what error happened and if it was caused by anything
        writeln!(f, "{red}{}{reset}", self.source)?;
        let mut cause = self.source.source();
        while let Some(error) = cause {
            writeln!(f, "  caused by: {error}")?;
            cause = error.source();
        }
        write!(f, "  {blue}-->{reset} {file}:{line}:{column}")?;

        // Get the file contents if we can to be highlighted, if we can't then
        // return early as we have at least rendered the error
        let Some(mut contents) = read_source(file) else {
            return Ok(());
        };
        // Color the code if we can
        if color {
            contents = highlight(&contents).unwrap_or(contents);
        }

        // Render the lines we actually want with line numbers and not going over
        // the length of the file itself
        let last = line.saturating_add(context).min(contents.lines().count());
        let shown = line.saturating_sub(context)..=last;
        let width = shown.end().to_string().len();
        write!(f, "\n{blue}{:width$} |{reset}", "")?;
        for (number, text) in (1..).zip(contents.lines()) {
            if !shown.contains(&number) {
                continue;
            }
            write!(f, "\n{blue}{number:width$} |{reset} {text}")?;
            // This renders the line showing WHERE the error ocurred
            if number == line {
                write!(
                    f,
                    "\n{blue}{:width$} |{reset} {red}{:>column$}{reset}",
                    "", "^"
                )?;
            }
        }
        Ok(())
    }
}

/// Use the default CONTEXT_LINES value if the env var is not set
fn context_lines() -> usize {
    var(CONTEXT_LINES_VAR)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(CONTEXT_LINES)
}

/// Set whether the coloring output for the error message should be used
fn use_color() -> bool {
    let cargo_color = var("CARGO_TERM_COLOR").ok();
    if var_os("NO_COLOR").is_some() || cargo_color.as_deref() == Some("never") {
        return false;
    }
    let forced = var_os("CLICOLOR_FORCE").is_some_and(|value| value != "0")
        || cargo_color.as_deref() == Some("always")
        || var_os("NEXTEST").is_some();
    forced || io::stderr().is_terminal()
}

/// Read the file where a test fails if it exists and can be read
fn read_source(file: &str) -> Option<String> {
    let cwd = current_dir().ok()?;
    cwd.ancestors()
        .find_map(|dir| read_to_string(dir.join(file)).ok())
}

/// Highlight the Rust source code handed to this function
fn highlight(source: &str) -> Option<String> {
    let mut config = HighlightConfiguration::new(
        LANGUAGE.into(),
        "rust",
        HIGHLIGHTS_QUERY,
        INJECTIONS_QUERY,
        "",
    )
    .ok()?;

    // Pass all possible types we might want to color to the config
    let names: Vec<&str> = PALETTE.iter().map(|(name, _)| *name).collect();
    config.configure(&names);

    // Setup the actual highlighter, we only care for rust here
    let mut highlighter = Highlighter::new();
    let events = highlighter
        .highlight(&config, source.as_bytes(), None, None, |language| {
            (language == "rust").then_some(&config)
        })
        .ok()?;

    let mut active = Vec::new();
    let mut output = String::new();
    for event in events {
        match event.ok()? {
            // Since highlighting can nest we need to maintain an active list of what can
            // be highlighted with color. index corresponds to a specific type in `PALETTE`
            // In most cases this is only a stack of 1, but not always
            HighlightEvent::HighlightStart(Highlight(index)) => active.push(PALETTE[index].1),
            // This current highlight can be dropped from the stack
            HighlightEvent::HighlightEnd => drop(active.pop()),
            // This part is where we actually color our code over a given part of the source
            // code and then add it to our final output
            HighlightEvent::Source { start, end } => {
                for (index, piece) in source.get(start..end)?.split('\n').enumerate() {
                    if index > 0 {
                        output.push('\n');
                    }
                    match active.iter().rev().find(|color| !color.is_empty()) {
                        Some(color) if !piece.is_empty() => output.extend([color, piece, RESET]),
                        _ => output.push_str(piece),
                    }
                }
            }
        }
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::read_to_string, num::ParseIntError};

    /// If the tree-sitter-grammar for rust is updated we want to make sure we cover every possible
    /// type in `PALETTE`
    #[test]
    fn palette_covers_every_highlight_the_grammar_emits() {
        let captures = HIGHLIGHTS_QUERY.split('@').skip(1).map(|rest| {
            rest.split(|c: char| c != '.' && !c.is_alphanumeric())
                .next()
                .unwrap()
        });
        for capture in captures {
            let covered = PALETTE.iter().any(|(name, _)| {
                capture
                    .strip_prefix(name)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('.'))
            });
            assert!(covered, "no palette entry for @{capture}");
        }
    }

    #[derive(Debug)]
    struct InvalidPort {
        source: ParseIntError,
    }

    impl fmt::Display for InvalidPort {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "invalid port")
        }
    }

    impl StdError for InvalidPort {
        fn source(&self) -> Option<&(dyn StdError + 'static)> {
            Some(&self.source)
        }
    }

    #[derive(Debug)]
    struct ConfigError {
        source: InvalidPort,
    }

    impl fmt::Display for ConfigError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "failed to load config")
        }
    }

    impl StdError for ConfigError {
        fn source(&self) -> Option<&(dyn StdError + 'static)> {
            Some(&self.source)
        }
    }

    fn load_config(port: &str) -> Result<u16, ConfigError> {
        let port = port.parse().map_err(|source| InvalidPort { source });
        port.map_err(|source| ConfigError { source })
    }

    #[test]
    #[ignore = "fails on purpose to show the output"]
    fn demo_missing_file() -> Result {
        let contents = read_to_string("/definitely/not/here")?;
        assert!(contents.is_empty());
        Ok(())
    }

    #[test]
    #[ignore = "fails on purpose to show the output"]
    fn demo_bad_number() -> Result {
        let port = "eighty".parse::<u16>()?;
        assert_eq!(port, 80);
        Ok(())
    }

    #[test]
    #[ignore = "fails on purpose to show the output"]
    fn demo_caused_by() -> Result {
        let port = load_config("eighty")?;
        assert_eq!(port, 80);
        Ok(())
    }
}
