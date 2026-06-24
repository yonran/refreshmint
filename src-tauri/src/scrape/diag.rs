//! Non-panicking stdout/stderr diagnostics for the long-lived scrape path.
//!
//! `print!`/`println!`/`eprint!`/`eprintln!` PANIC if the write fails — e.g. a
//! broken pipe (EPIPE). Auto-scrape runs inside a long-lived process whose
//! stdout/stderr reader can disappear mid-session: notably under `tauri dev`,
//! which pipes the app's output through the dev server to a terminal that may
//! go away. A panic from a stray diagnostic then aborts the whole spawned
//! scrape task with "failed printing to stderr: Broken pipe", killing the
//! scrape (observed crashing every auto-scrape from 2026-06-13 on).
//!
//! These helpers write the same text to the same stream as the std macros but
//! swallow I/O errors, so a dead pipe can never crash a scrape. Use the
//! `diag_eprintln!` / `diag_eprint!` / `diag_println!` macros as drop-in
//! replacements for `eprintln!` / `eprint!` / `println!` in scrape code.

use std::io::Write;

/// Write `args` followed by a newline to `w`, ignoring any I/O error.
pub(crate) fn write_line(mut w: impl Write, args: std::fmt::Arguments<'_>) {
    let _ = writeln!(w, "{args}");
}

/// Write `args` to `w` with no trailing newline, ignoring any I/O error.
pub(crate) fn write_str(mut w: impl Write, args: std::fmt::Arguments<'_>) {
    let _ = write!(w, "{args}");
}

macro_rules! diag_eprintln {
    ($($arg:tt)*) => {
        $crate::scrape::diag::write_line(std::io::stderr(), format_args!($($arg)*))
    };
}

macro_rules! diag_eprint {
    ($($arg:tt)*) => {
        $crate::scrape::diag::write_str(std::io::stderr(), format_args!($($arg)*))
    };
}

macro_rules! diag_println {
    ($($arg:tt)*) => {
        $crate::scrape::diag::write_line(std::io::stdout(), format_args!($($arg)*))
    };
}

pub(crate) use {diag_eprint, diag_eprintln, diag_println};

#[cfg(test)]
mod tests {
    use super::*;

    /// A writer that always fails with `BrokenPipe`, like a closed stdout/stderr
    /// pipe (EPIPE). The std print macros would panic on this; our helpers must not.
    struct BrokenWriter;
    impl Write for BrokenWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "broken pipe",
            ))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "broken pipe",
            ))
        }
    }

    #[test]
    fn write_helpers_do_not_panic_on_broken_pipe() {
        // Both must return normally even though every write errors with EPIPE.
        write_line(BrokenWriter, format_args!("hello {}", 1));
        write_str(BrokenWriter, format_args!("hello {}", 2));
    }

    #[test]
    fn write_line_appends_newline() {
        let mut buf: Vec<u8> = Vec::new();
        write_line(&mut buf, format_args!("a={}", 7));
        assert_eq!(buf, b"a=7\n");
    }
}
