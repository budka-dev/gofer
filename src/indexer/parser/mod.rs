pub mod lang_manager;
pub mod chunking;
#[allow(unused_imports)]
pub mod core;
pub mod skeleton;
pub mod type_fields;

use std::sync::{Arc, LazyLock};
use std::cell::RefCell;
use tree_sitter::Parser;

/// Global LanguageManager for dynamic parser loading.
pub static LANG_MANAGER: LazyLock<Arc<lang_manager::LanguageManager>> = LazyLock::new(|| {
    Arc::new(lang_manager::LanguageManager::new(None, None).expect("Failed to initialize LanguageManager"))
});

thread_local! {
    static THREAD_PARSER: RefCell<Option<Parser>> = const { RefCell::new(None) };
}

pub fn with_parser<F, R>(f: F) -> R 
where 
    F: FnOnce(&mut Parser) -> R 
{
    THREAD_PARSER.with(|cell| {
        let mut borrow = cell.borrow_mut();
        if borrow.is_none() {
            let mut parser = Parser::new();
            if let Ok(store) = tree_sitter::WasmStore::new(&LANG_MANAGER.engine) {
                let _ = parser.set_wasm_store(store);
            }
            *borrow = Some(parser);
        }
        f(borrow.as_mut().unwrap())
    })
}

// Реэкспорт публичного API — потребители не меняются
#[allow(unused_imports)]
pub use self::core::{CodeParser, ParsedFile, ParserError, Result, SupportedLanguage};
#[allow(unused_imports)]
pub use chunking::smart_chunk_file;
#[allow(unused_imports)]
pub use skeleton::generate_skeleton;
#[allow(unused_imports)]
pub use type_fields::{normalize_field, parse_all_type_fields, parse_type_fields};
