use std::io::{self, IsTerminal};

#[derive(Copy, Clone)]
pub enum Color {
    Red,
    Yellow,
    Green,
    Blue,
}

pub fn color_enabled() -> bool {
    match std::env::var("NO_COLOR") {
        Ok(v) if !v.is_empty() => return false,
        _ => {}
    }

    match std::env::var("FORCE_COLOR") {
        Ok(v) if !v.is_empty() && v != "0" => return true,
        _ => {}
    }

    io::stdout().is_terminal() || io::stderr().is_terminal()
}

pub fn paint(text: impl AsRef<str>, color: Color) -> String {
    let text = text.as_ref();
    if !color_enabled() {
        return text.to_owned();
    }

    let code = match color {
        Color::Red => 31,
        Color::Yellow => 33,
        Color::Green => 32,
        Color::Blue => 34,
    };

    format!("\x1b[{code}m{text}\x1b[0m")
}

pub fn error(text: impl AsRef<str>) -> String {
    paint(text, Color::Red)
}

pub fn warning(text: impl AsRef<str>) -> String {
    paint(text, Color::Yellow)
}

pub fn highlight(text: impl AsRef<str>) -> String {
    paint(text, Color::Green)
}

pub fn info(text: impl AsRef<str>) -> String {
    paint(text, Color::Blue)
}

pub fn shell_printf(text: &str, color: Option<Color>) -> String {
    let text = text.replace('\'', "'\\''");
    let format = match color.filter(|_| color_enabled()) {
        Some(Color::Red) => "\\033[31m%s\\033[0m\\n",
        Some(Color::Yellow) => "\\033[33m%s\\033[0m\\n",
        Some(Color::Green) => "\\033[32m%s\\033[0m\\n",
        Some(Color::Blue) => "\\033[34m%s\\033[0m\\n",
        None => "%s\\n",
    };

    format!("printf '{format}' '{text}'")
}

pub fn shell_printf_inline(text: &str, color: Option<Color>) -> String {
    let text = text.replace('\'', "'\\''");
    let format = match color.filter(|_| color_enabled()) {
        Some(Color::Red) => "\\033[31m%s\\033[0m",
        Some(Color::Yellow) => "\\033[33m%s\\033[0m",
        Some(Color::Green) => "\\033[32m%s\\033[0m",
        Some(Color::Blue) => "\\033[34m%s\\033[0m",
        None => "%s",
    };

    format!("printf '{format}' '{text}'")
}
