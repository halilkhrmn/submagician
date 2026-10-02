//! SubMagician core: everything that does not need a window.
//!
//! Scanning folders, identifying videos (hash + file name), asking subtitle providers,
//! ranking what they return, and saving the chosen subtitle next to the video as UTF-8.

pub mod archive;
pub mod engine;
pub mod error;
pub mod hash;
pub mod lang;
pub mod media;
pub mod name;
pub mod output;
pub mod provider;
pub mod score;
pub mod text;

pub use error::{Error, Result};
