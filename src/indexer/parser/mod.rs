pub mod lang_manager;
pub mod chunking;
#[allow(unused_imports)]
pub mod core;
pub mod skeleton;
pub mod type_fields;

// Реэкспорт публичного API — потребители не меняются
#[allow(unused_imports)]
pub use self::core::{CodeParser, ParsedFile, ParserError, Result, SupportedLanguage};
#[allow(unused_imports)]
pub use chunking::smart_chunk_file;
#[allow(unused_imports)]
pub use skeleton::generate_skeleton;
#[allow(unused_imports)]
pub use type_fields::{normalize_field, parse_all_type_fields, parse_type_fields};
