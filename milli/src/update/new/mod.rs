pub use document_change::{Deletion, DocumentChange, Insertion, Update};
pub use items_pool::ItemsPool;
pub use top_level_map::{CowStr, TopLevelMap};

use super::del_add::DelAdd;
use crate::FieldId;

mod append_only_vec;
mod channel;
mod document_change;
mod extract;
pub mod indexer;
mod items_pool;
mod merger;
mod top_level_map;
mod word_fst_builder;
mod words_prefix_docids;

/// TODO move them elsewhere
pub type StdResult<T, E> = std::result::Result<T, E>;
pub type KvReaderDelAdd = obkv::KvReader<DelAdd>;
pub type KvReaderFieldId = obkv::KvReader<FieldId>;
pub type KvWriterDelAdd<W> = obkv::KvWriter<W, DelAdd>;
pub type KvWriterFieldId<W> = obkv::KvWriter<W, FieldId>;
