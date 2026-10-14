//! Command groups added per ribbon tab; each file owns its operations and their tests.
//! Add one `mod` line per file; other agents append theirs.

pub mod data;
pub(crate) mod data_table;
pub mod header_footer;
pub mod insert_more;
pub mod mailings;
pub mod references_more;
pub mod review;
pub mod sheet_objects;
pub mod sheet_page;
pub mod sheets_home;
pub mod writer_design;
pub mod writer_home;
pub mod writer_insert;
pub mod writer_layout;
pub mod writer_margins;
pub mod writer_references;
pub mod writer_review;
pub mod writer_search;
pub(crate) mod writer_xml;
pub(crate) mod xmlpatch;
