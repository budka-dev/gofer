-- Content-level FTS for hybrid search (keyword match on chunk bodies, not only symbols).
CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
    file_path,
    content,
    line_start UNINDEXED,
    line_end UNINDEXED,
    tokenize = 'porter unicode61'
);
