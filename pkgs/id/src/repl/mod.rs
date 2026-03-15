//! REPL module - interactive command-line interface

pub mod input;

pub use input::{continue_heredoc, preprocess_repl_line, shell_capture, ReplInput};
