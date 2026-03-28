//! qndx-index: index builder, storage writer/reader.

pub mod ngram;
pub mod postings;
pub mod builder;
pub mod reader;

pub use ngram::*;
pub use builder::{build_index, build_index_from_dir, BuildResult};
pub use reader::IndexReader;
