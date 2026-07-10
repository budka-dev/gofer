-- Drop knowledge/analysis tables no longer used by the index-search MCP surface.
-- Historical migrations remain; this cleans live DBs.

DROP TABLE IF EXISTS dependency_usage;
DROP TABLE IF EXISTS dependencies;
DROP TABLE IF EXISTS rules;
DROP TABLE IF EXISTS golden_samples;
DROP TABLE IF EXISTS active_errors;
DROP TABLE IF EXISTS config_keys;
DROP TABLE IF EXISTS vue_trees;
DROP TABLE IF EXISTS type_fields;
DROP TABLE IF EXISTS entity_links;
DROP TABLE IF EXISTS api_endpoints;
DROP TABLE IF EXISTS frontend_api_calls;
DROP TABLE IF EXISTS type_fingerprints;
DROP TABLE IF EXISTS cross_stack_links;
DROP TABLE IF EXISTS file_summaries;
DROP TABLE IF EXISTS summary_queue;
