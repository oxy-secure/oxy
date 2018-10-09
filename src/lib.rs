//! Oxy is a remote access tool designed to resist man-in-the-middle attacks
//! irrespective of user dilligence. This is an experimental version. Do not
//! use outside of an isolated lab environment.

#![warn(missing_docs)]
#![feature(int_to_from_bytes)]

#[macro_use]
extern crate serde_derive;

pub mod arg;
pub mod config;
pub mod entry;
pub mod oxy;

mod base32;
mod inner;
mod log;
mod outer;
mod oxyhelpers;
