use gofer::indexer::parser::{CodeParser, SupportedLanguage};

#[test]
fn test_debug_wasm_parsing() {
    let mut parser = CodeParser::new();
    let code = "fn main() { println!(\"hello\"); }";
    let lang = SupportedLanguage(SupportedLanguage::RUST.to_string());
    
    match parser.parse_file(code, "test.rs", lang) {
        Ok(parsed) => {
            println!("Symbols: {}", parsed.symbols.len());
            println!("Chunks: {}", parsed.chunks.len());
            println!("Refs: {}", parsed.refs.len());
            
            if parsed.symbols.is_empty() {
                println!("WARNING: AST parsed but 0 symbols extracted. Query mismatch or empty AST?");
            }
        }
        Err(e) => {
            println!("Parse failed: {:?}", e);
        }
    }
}
