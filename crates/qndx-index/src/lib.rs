//! qndx-index: index builder, storage writer/reader.

pub mod builder;
pub mod ngram;
pub mod overlay;
pub mod postings;
pub mod reader;

pub use builder::{BuildResult, build_index, build_index_from_dir};
pub use ngram::*;
pub use overlay::OverlayIndex;
pub use reader::IndexReader;
