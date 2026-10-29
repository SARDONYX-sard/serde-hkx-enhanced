//! Terminal output: color markup, `--quiet`, and progress bars.
//!
//! # Color markup
//!
//! Help texts and messages use a small tag syntax rendered by [`render`]:
//!
//! ```text
//! <cyan!>hkxc exportrig</cyan!> -a <<FILE>>
//! ```
//!
//! - `<red>`, `<green>`, `<yellow>`, `<blue>`, `<cyan>`, `<white>`: colors,
//!   `!` suffix for the bright variant (`<blue!>`);
//! - `<bold>`, `<underline>`;
//! - `<<` and `>>` are literal `<` and `>`.
//!
//! Colors are only emitted when the stream is a terminal (and `NO_COLOR` is
//! not set), so `hkxc --help > help.txt` writes plain text.

use std::io::IsTerminal as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};

static QUIET: AtomicBool = AtomicBool::new(false);

/// Hides progress bars and informational output (`-q/--quiet`).
pub fn set_quiet(quiet: bool) {
    QUIET.store(quiet, Ordering::Relaxed);
}

pub fn quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}

#[derive(Debug, Clone, Copy)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// Whether ANSI colors should be written to `stream`.
pub fn color_enabled(stream: Stream) -> bool {
    if !cfg!(feature = "color") || std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        return false;
    }
    match stream {
        Stream::Stdout => std::io::stdout().is_terminal(),
        Stream::Stderr => std::io::stderr().is_terminal(),
    }
}

/// Markup rendered with ANSI colors.
///
/// For clap help texts: clap removes the escape codes itself when its output
/// is not a terminal.
pub fn paint(markup: &str) -> String {
    render(markup, cfg!(feature = "color"))
}

/// Markup rendered for `stream` (plain text when it is not a terminal).
pub fn paint_for(stream: Stream, markup: &str) -> String {
    render(markup, color_enabled(stream))
}

/// Escapes `<` / `>` so that `text` is shown literally inside markup.
pub fn escape(text: &str) -> String {
    text.replace('<', "<<").replace('>', ">>")
}

/// `[Error]` message for stderr.
pub fn error(message: &str) -> String {
    paint_for(
        Stream::Stderr,
        &format!("<red>[Error]\n{}</red>", escape(message)),
    )
}

/// Renders markup; with `color == false` the tags are removed.
pub fn render(markup: &str, color: bool) -> String {
    let mut out = String::with_capacity(markup.len());
    let mut stack: Vec<&'static str> = Vec::new();
    let mut rest = markup;

    while let Some(i) = rest.find(['<', '>']) {
        out.push_str(&rest[..i]);
        rest = &rest[i..];

        if let Some(after) = rest.strip_prefix("<<") {
            out.push('<');
            rest = after;
            continue;
        }
        if let Some(after) = rest.strip_prefix(">>") {
            out.push('>');
            rest = after;
            continue;
        }

        // `<tag>` / `</tag>`
        let tag = rest.strip_prefix('<').and_then(|t| t.split_once('>'));
        let Some((tag, after)) = tag else {
            out.push_str(&rest[..1]);
            rest = &rest[1..];
            continue;
        };
        let (closing, name) = tag
            .strip_prefix('/')
            .map_or((false, tag), |name| (true, name));
        let Some(code) = ansi_code(name) else {
            // Not a known tag: keep it as text.
            out.push_str(&rest[..tag.len() + 2]);
            rest = after;
            continue;
        };

        if closing {
            stack.pop();
            if color {
                out.push_str("\x1b[0m");
                for code in &stack {
                    out.push_str(code);
                }
            }
        } else {
            stack.push(code);
            if color {
                out.push_str(code);
            }
        }
        rest = after;
    }
    out.push_str(rest);

    if color && !stack.is_empty() {
        out.push_str("\x1b[0m");
    }
    out
}

fn ansi_code(tag: &str) -> Option<&'static str> {
    Some(match tag {
        "bold" => "\x1b[1m",
        "underline" => "\x1b[4m",
        "red" => "\x1b[31m",
        "green" => "\x1b[32m",
        "yellow" => "\x1b[33m",
        "blue" => "\x1b[34m",
        "cyan" => "\x1b[36m",
        "white" => "\x1b[37m",
        "red!" => "\x1b[91m",
        "green!" => "\x1b[92m",
        "yellow!" => "\x1b[93m",
        "blue!" => "\x1b[94m",
        "cyan!" => "\x1b[96m",
        "white!" => "\x1b[97m",
        _ => return None,
    })
}

/// Prints an informational line to stdout unless `--quiet`.
pub fn info(markup: &str) {
    if !quiet() {
        println!("{}", paint_for(Stream::Stdout, markup));
    }
}

/// A progress bar on stderr; hidden with `--quiet` or when stderr is not a terminal.
#[expect(
    clippy::literal_string_with_formatting_args,
    reason = "indicatif template"
)]
pub fn progress_bar(total: u64, prefix: impl Into<String>) -> ProgressBar {
    if quiet() || !std::io::stderr().is_terminal() {
        return ProgressBar::hidden();
    }
    let bar = ProgressBar::new(total).with_prefix(prefix.into());
    if let Ok(style) = ProgressStyle::with_template(
        "{spinner:.green} {prefix:.bold} [{elapsed_precise}] {bar:30.cyan/blue} {pos}/{len} {wide_msg}",
    ) {
        bar.set_style(style.progress_chars("#>-"));
    }
    bar.enable_steady_tick(Duration::from_millis(120));
    bar
}

/// A spinner for a step of unknown length; hidden like [`progress_bar`].
pub fn spinner(message: impl Into<String>) -> ProgressBar {
    if quiet() || !std::io::stderr().is_terminal() {
        return ProgressBar::hidden();
    }
    let bar = ProgressBar::new_spinner().with_message(message.into());
    if let Ok(style) = ProgressStyle::with_template("{spinner:.green} {wide_msg}") {
        bar.set_style(style);
    }
    bar.enable_steady_tick(Duration::from_millis(120));
    bar
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_markup() {
        let markup = "<cyan!>hkxc</cyan!> -o <<OUT>> <bold><red>x</red>y</bold> <foo>";
        assert_eq!(render(markup, false), "hkxc -o <OUT> xy <foo>");
        assert_eq!(
            render("<bold><red>x</red>y</bold>", true),
            "\x1b[1m\x1b[31mx\x1b[0m\x1b[1my\x1b[0m"
        );
    }
}
