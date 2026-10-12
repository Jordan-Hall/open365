//! Test support for the clean-room preset measurement (docs/architecture/drawing.md).
//!
//! `pdf` reads the vector paths (`m l c v y h re` painted by `S s f B ...`) out of a Word PDF
//! export; `geom` flattens paths and measures the largest deviation between two sets of them.
//! Neither is a general PDF or geometry library.
#![allow(dead_code)]

pub mod geom;
pub mod pdf;
