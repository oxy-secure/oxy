//! Oxy is a remote access tool designed to resist man-in-the-middle attacks
//! irrespective of user dilligence.

#![warn(missing_docs)]

#[macro_use]
extern crate serde_derive;

pub mod arg;
mod base32;
pub mod config;
pub mod entry;
mod log;
pub mod oxy;
