//! qndx-index: index builder, storage writer/reader.

pub mod builder;
pub mod ngram;
pub mod postings;
pub mod reader;

pub use builder::{build_index, build_index_from_dir, BuildResult};
pub use ngram::*;
pub use reader::IndexReader;
