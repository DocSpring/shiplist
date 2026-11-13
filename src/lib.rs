//! shiplist: every file in a repository goes in exactly one list, so CI decides what each build stage gets.
//!
//! See README.md for the config format and the commands.

pub mod archive;
pub mod check;
pub mod classify;
pub mod config;
pub mod image;
pub mod pattern;
pub mod prune;
pub mod tree;
pub mod verify;
