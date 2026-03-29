//! rust-analyzer LSP client for advanced Rust language support.
//!
//! Provides deep Rust code intelligence via rust-analyzer:
//! - Go to definition / Find references
//! - Hover information (types, docs)
//! - Diagnostics (errors, warnings)
//! - Code completions
//! - Inlay hints (types, parameter names)
//! - Code actions (quick fixes, refactorings)

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::str::FromStr;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use lsp_types::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{oneshot, Mutex, RwLock};
use tracing::{debug, error, info, warn};

type ResponseSender = oneshot::Sender<Result<Value>>;

/// Generic LSP client wrapper.
pub struct GenericLspClient {
    /// Target LSP executable command
    command: String,
    /// Arguments for the LSP executable
    args: Vec<String>,
    /// Language ID for textDocument objects (e.g. "rust", "python")
    lang_id: String,
    /// Project root path
    root_path: PathBuf,
    /// LSP initialisation options
    init_options: Option<Value>,
    /// rust-analyzer process handle
    process: Arc<Mutex<Option<Child>>>,
    /// Stdin handle for sending requests
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    /// Next request ID
    next_id: Arc<RwLock<i32>>,
    /// Initialization status
    initialized: Arc<RwLock<bool>>,
    /// Pending requests waiting for responses
    pending_requests: Arc<RwLock<HashMap<i32, ResponseSender>>>,
    /// Collected diagnostics per file
    diagnostics: Arc<RwLock<HashMap<String, Vec<Diagnostic>>>>,
}

impl GenericLspClient {
    /// Create a new generic LSP client for the given project.
    pub fn new(root_path: PathBuf, command: String, args: Vec<String>, lang_id: String, init_options: Option<Value>) -> Self {
        Self {
            command,
            args,
            lang_id,
            root_path,
            init_options,
            process: Arc::new(Mutex::new(None)),
            stdin: Arc::new(Mutex::new(None)),
            next_id: Arc::new(RwLock::new(1)),
            initialized: Arc::new(RwLock::new(false)),
            pending_requests: Arc::new(RwLock::new(HashMap::new())),
            diagnostics: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Start LSP server process and initialize.
    pub async fn start(&self) -> Result<()> {
        let mut proc_guard = self.process.lock().await;

        if proc_guard.is_some() {
            warn!("LSP server {} already running", self.command);
            return Ok(());
        }

        info!("Starting LSP server {} for {:?}", self.command, self.root_path);

        let mut child = Command::new(&self.command)
            .args(&self.args)
            .current_dir(&self.root_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .context(format!("Failed to spawn LSP server {}", self.command))?;

        let stdin = child.stdin.take().context("Failed to get stdin")?;
        let stdout = child.stdout.take().context("Failed to get stdout")?;

        *proc_guard = Some(child);
        drop(proc_guard);

        // Store stdin for future requests
        *self.stdin.lock().await = Some(stdin);

        // Spawn background task to read responses
        self.spawn_response_reader(stdout);

        // Send initialize request
        let initialize_params = InitializeParams {
            process_id: Some(std::process::id()),
            workspace_folders: Some(vec![WorkspaceFolder {
                uri: Uri::from_str(&format!("file://{}", self.root_path.display()))
                    .map_err(|_| anyhow::anyhow!("Invalid root path"))?,
                name: self
                    .root_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("workspace")
                    .to_string(),
            }]),
            capabilities: ClientCapabilities {
                text_document: Some(TextDocumentClientCapabilities {
                    hover: Some(HoverClientCapabilities {
                        content_format: Some(vec![MarkupKind::Markdown, MarkupKind::PlainText]),
                        ..Default::default()
                    }),
                    completion: Some(CompletionClientCapabilities {
                        completion_item: Some(CompletionItemCapability {
                            snippet_support: Some(true),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    definition: Some(GotoCapability {
                        link_support: Some(false),
                        ..Default::default()
                    }),
                    references: Some(ReferenceClientCapabilities {
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            initialization_options: self.init_options.clone(),
            ..Default::default()
        };

        let _response: InitializeResult =
            self.send_request("initialize", initialize_params).await?;

        // Send initialized notification
        self.send_notification("initialized", InitializedParams {})
            .await?;

        *self.initialized.write().await = true;
        info!("LSP server initialized successfully");

        Ok(())
    }

    /// Spawn background task to read and dispatch responses.
    fn spawn_response_reader(&self, stdout: ChildStdout) {
        let pending_requests = self.pending_requests.clone();
        let diagnostics = self.diagnostics.clone();

        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);

            loop {
                match Self::read_message(&mut reader).await {
                    Ok(Some(msg)) => {
                        if let Err(e) =
                            Self::handle_message(msg, &pending_requests, &diagnostics).await
                        {
                            error!("Failed to handle LSP message: {}", e);
                        }
                    }
                    Ok(None) => {
                        debug!("LSP server stdout closed");
                        pending_requests.write().await.clear();
                        break;
                    }
                    Err(e) => {
                        error!("Failed to read LSP message: {}", e);
                        pending_requests.write().await.clear();
                        break;
                    }
                }
            }
        });
    }

    /// Read a single LSP message from stdout.
    async fn read_message(reader: &mut BufReader<ChildStdout>) -> Result<Option<Value>> {
        let mut headers = Vec::new();

        loop {
            let mut line = String::new();
            let n = reader.read_line(&mut line).await?;

            if n == 0 {
                return Ok(None); // EOF
            }

            if line == "\r\n" {
                break;
            }
            headers.push(line);
        }

        let content_length = headers
            .iter()
            .find_map(|h| {
                h.strip_prefix("Content-Length: ")
                    .and_then(|s| s.trim().parse::<usize>().ok())
            })
            .context("Missing Content-Length header")?;

        let mut content_buf = vec![0u8; content_length];
        tokio::io::AsyncReadExt::read_exact(reader, &mut content_buf).await?;

        let msg: Value = serde_json::from_slice(&content_buf)?;
        Ok(Some(msg))
    }

    /// Handle an incoming LSP message (response or notification).
    async fn handle_message(
        msg: Value,
        pending_requests: &Arc<RwLock<HashMap<i32, ResponseSender>>>,
        diagnostics: &Arc<RwLock<HashMap<String, Vec<Diagnostic>>>>,
    ) -> Result<()> {
        // Check if it's a response (has "id" field)
        if let Some(id_value) = msg.get("id") {
            if let Some(id) = id_value.as_i64() {
                let mut pending = pending_requests.write().await;

                if let Some(sender) = pending.remove(&(id as i32)) {
                    if let Some(error) = msg.get("error") {
                        let _ = sender.send(Err(anyhow::anyhow!("LSP error: {}", error)));
                    } else if let Some(result) = msg.get("result") {
                        let _ = sender.send(Ok(result.clone()));
                    } else {
                        let _ = sender.send(Err(anyhow::anyhow!("Invalid LSP response")));
                    }
                }
            }
        }
        // Check if it's a notification
        else if let Some(method) = msg.get("method").and_then(|m| m.as_str()) {
            match method {
                "textDocument/publishDiagnostics" => {
                    if let Some(params) = msg.get("params") {
                        Self::handle_diagnostics(params, diagnostics).await?;
                    }
                }
                _ => {
                    debug!("Unhandled notification: {}", method);
                }
            }
        }

        Ok(())
    }

    /// Handle incoming diagnostics notification.
    async fn handle_diagnostics(
        params: &Value,
        diagnostics: &Arc<RwLock<HashMap<String, Vec<Diagnostic>>>>,
    ) -> Result<()> {
        let notification: PublishDiagnosticsParams = serde_json::from_value(params.clone())?;
        let file_path = notification.uri.path().to_string();

        let mut diag_map = diagnostics.write().await;
        diag_map.insert(file_path, notification.diagnostics);

        Ok(())
    }

    /// Stop rust-analyzer process.
    pub async fn stop(&self) -> Result<()> {
        let mut proc_guard = self.process.lock().await;

        if let Some(mut child) = proc_guard.take() {
            info!("Stopping LSP server");
            child.kill().await.context("Failed to kill LSP server")?;
            *self.initialized.write().await = false;
            *self.stdin.lock().await = None;
            self.pending_requests.write().await.clear();
        }

        Ok(())
    }

    /// Check if rust-analyzer is running and initialized.
    pub async fn is_ready(&self) -> bool {
        *self.initialized.read().await
    }

    /// Send LSP request and wait for response.
    async fn send_request<P: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        params: P,
    ) -> Result<R> {
        let id = {
            let mut next_id = self.next_id.write().await;
            let id = *next_id;
            *next_id += 1;
            id
        };

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        // Create oneshot channel for response
        let (tx, rx) = oneshot::channel();

        // Register pending request
        self.pending_requests.write().await.insert(id, tx);

        // Send request
        let content = serde_json::to_string(&request)?;
        let message = format!("Content-Length: {}\r\n\r\n{}", content.len(), content);

        let mut stdin_guard = self.stdin.lock().await;
        let stdin = stdin_guard.as_mut().context("LSP server not running")?;
        stdin.write_all(message.as_bytes()).await?;
        stdin.flush().await?;
        drop(stdin_guard);

        // Wait for response with timeout
        let result = tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .context("LSP request timeout")??;

        Ok(serde_json::from_value(result?)?)
    }

    /// Send LSP notification (no response expected).
    async fn send_notification<P: Serialize>(&self, method: &str, params: P) -> Result<()> {
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });

        let content = serde_json::to_string(&notification)?;
        let message = format!("Content-Length: {}\r\n\r\n{}", content.len(), content);

        let mut stdin_guard = self.stdin.lock().await;
        let stdin = stdin_guard.as_mut().context("LSP server not running")?;
        stdin.write_all(message.as_bytes()).await?;
        stdin.flush().await?;

        Ok(())
    }

    /// Notify LSP server that a file was opened.
    pub async fn did_open(&self, file_path: &Path, content: String) -> Result<()> {
        let uri = Uri::from_str(&format!("file://{}", file_path.display()))
            .map_err(|e| anyhow::anyhow!("Invalid file path: {}", e))?;

        let params = DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri,
                language_id: self.lang_id.clone(),
                version: 1,
                text: content,
            },
        };

        self.send_notification("textDocument/didOpen", params).await
    }

    /// Notify rust-analyzer that a file was changed.
    #[allow(dead_code)]
    pub async fn did_change(&self, file_path: &Path, content: String, version: i32) -> Result<()> {
        let uri = Uri::from_str(&format!("file://{}", file_path.display()))
            .map_err(|e| anyhow::anyhow!("Invalid file path: {}", e))?;

        let params = DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier { uri, version },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: content,
            }],
        };

        self.send_notification("textDocument/didChange", params)
            .await
    }

    /// Go to definition for a symbol at position.
    pub async fn goto_definition(
        &self,
        file_path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<Location>> {
        if !self.is_ready().await {
            bail!("LSP server not initialized");
        }

        let params = GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: Uri::from_str(&format!("file://{}", file_path.display()))
                        .map_err(|e| anyhow::anyhow!("Invalid file path: {}", e))?,
                },
                position: Position { line, character },
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };

        let response: Option<GotoDefinitionResponse> =
            self.send_request("textDocument/definition", params).await?;

        match response {
            Some(GotoDefinitionResponse::Scalar(loc)) => Ok(vec![loc]),
            Some(GotoDefinitionResponse::Array(locs)) => Ok(locs),
            Some(GotoDefinitionResponse::Link(links)) => Ok(links
                .into_iter()
                .map(|link| Location {
                    uri: link.target_uri,
                    range: link.target_selection_range,
                })
                .collect()),
            None => Ok(vec![]),
        }
    }

    /// Find all references to symbol at position.
    pub async fn find_references(
        &self,
        file_path: &Path,
        line: u32,
        character: u32,
        include_declaration: bool,
    ) -> Result<Vec<Location>> {
        if !self.is_ready().await {
            bail!("LSP server not initialized");
        }

        let params = ReferenceParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: Uri::from_str(&format!("file://{}", file_path.display()))
                        .map_err(|e| anyhow::anyhow!("Invalid file path: {}", e))?,
                },
                position: Position { line, character },
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
            context: ReferenceContext {
                include_declaration,
            },
        };

        let response: Option<Vec<Location>> =
            self.send_request("textDocument/references", params).await?;

        Ok(response.unwrap_or_default())
    }

    /// Get hover information at position.
    pub async fn hover(
        &self,
        file_path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Option<Hover>> {
        if !self.is_ready().await {
            bail!("LSP server not initialized");
        }

        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: Uri::from_str(&format!("file://{}", file_path.display()))
                        .map_err(|e| anyhow::anyhow!("Invalid file path: {}", e))?,
                },
                position: Position { line, character },
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
        };

        self.send_request("textDocument/hover", params).await
    }

    /// Get diagnostics for a file.
    pub async fn diagnostics(&self, file_path: &Path) -> Result<Vec<Diagnostic>> {
        let diag_map = self.diagnostics.read().await;
        let path_str = file_path.to_string_lossy().to_string();

        Ok(diag_map.get(&path_str).cloned().unwrap_or_default())
    }

    /// Get code completions at position.
    pub async fn completions(
        &self,
        file_path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<CompletionItem>> {
        if !self.is_ready().await {
            bail!("LSP server not initialized");
        }

        let params = CompletionParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: Uri::from_str(&format!("file://{}", file_path.display()))
                        .map_err(|e| anyhow::anyhow!("Invalid file path: {}", e))?,
                },
                position: Position { line, character },
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
            context: None,
        };

        let response: Option<CompletionResponse> =
            self.send_request("textDocument/completion", params).await?;

        match response {
            Some(CompletionResponse::Array(items)) => Ok(items),
            Some(CompletionResponse::List(list)) => Ok(list.items),
            None => Ok(vec![]),
        }
    }

    /// Get inlay hints for a range.
    pub async fn inlay_hints(
        &self,
        file_path: &Path,
        start_line: u32,
        end_line: u32,
    ) -> Result<Vec<InlayHint>> {
        if !self.is_ready().await {
            bail!("LSP server not initialized");
        }

        let params = InlayHintParams {
            text_document: TextDocumentIdentifier {
                uri: Uri::from_str(&format!("file://{}", file_path.display()))
                    .map_err(|e| anyhow::anyhow!("Invalid file path: {}", e))?,
            },
            range: Range {
                start: Position {
                    line: start_line,
                    character: 0,
                },
                end: Position {
                    line: end_line,
                    character: 0,
                },
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
        };

        let response: Option<Vec<InlayHint>> =
            self.send_request("textDocument/inlayHint", params).await?;

        Ok(response.unwrap_or_default())
    }

    /// Get code actions for a range.
    pub async fn code_actions(
        &self,
        file_path: &Path,
        start_line: u32,
        end_line: u32,
        diagnostics: Vec<Diagnostic>,
    ) -> Result<Vec<CodeActionOrCommand>> {
        if !self.is_ready().await {
            bail!("LSP server not initialized");
        }

        let params = CodeActionParams {
            text_document: TextDocumentIdentifier {
                uri: Uri::from_str(&format!("file://{}", file_path.display()))
                    .map_err(|e| anyhow::anyhow!("Invalid file path: {}", e))?,
            },
            range: Range {
                start: Position {
                    line: start_line,
                    character: 0,
                },
                end: Position {
                    line: end_line,
                    character: 0,
                },
            },
            context: CodeActionContext {
                diagnostics,
                only: None,
                trigger_kind: None,
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };

        let response: Option<Vec<CodeActionOrCommand>> =
            self.send_request("textDocument/codeAction", params).await?;

        Ok(response.unwrap_or_default())
    }

    /// Get document symbols (outline of structures, functions, etc.) for a file.
    pub async fn document_symbols(&self, file_path: &Path) -> Result<Vec<DocumentSymbol>> {
        if !self.is_ready().await {
            bail!("LSP server not initialized");
        }

        let params = DocumentSymbolParams {
            text_document: TextDocumentIdentifier {
                uri: Uri::from_str(&format!("file://{}", file_path.display()))
                    .map_err(|e| anyhow::anyhow!("Invalid file path: {}", e))?,
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };

        let response: Option<DocumentSymbolResponse> = self
            .send_request("textDocument/documentSymbol", params)
            .await?;

        match response {
            Some(DocumentSymbolResponse::Flat(symbols)) => {
                // Convert flat SymbolInformation to hierarchical DocumentSymbol
                Ok(symbols
                    .into_iter()
                    .map(|sym| DocumentSymbol {
                        name: sym.name.clone(),
                        detail: None,
                        kind: sym.kind,
                        tags: sym.tags,
                        range: sym.location.range,
                        selection_range: sym.location.range,
                        children: None,
                        #[allow(deprecated)]
                        deprecated: sym.deprecated,
                    })
                    .collect())
            }
            Some(DocumentSymbolResponse::Nested(symbols)) => Ok(symbols),
            None => Ok(vec![]),
        }
    }

    /// Search for symbols across the entire workspace.
    pub async fn workspace_symbols(&self, query: &str) -> Result<Vec<SymbolInformation>> {
        if !self.is_ready().await {
            bail!("LSP server not initialized");
        }

        let params = WorkspaceSymbolParams {
            query: query.to_string(),
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };

        let response: Option<Vec<SymbolInformation>> =
            self.send_request("workspace/symbol", params).await?;

        Ok(response.unwrap_or_default())
    }

    /// Go to implementation(s) of a trait method or type.
    pub async fn goto_implementation(
        &self,
        file_path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<Location>> {
        if !self.is_ready().await {
            bail!("LSP server not initialized");
        }

        let params = request::GotoImplementationParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: Uri::from_str(&format!("file://{}", file_path.display()))
                        .map_err(|e| anyhow::anyhow!("Invalid file path: {}", e))?,
                },
                position: Position { line, character },
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };

        let response: Option<GotoDefinitionResponse> = self
            .send_request("textDocument/implementation", params)
            .await?;

        match response {
            Some(GotoDefinitionResponse::Scalar(loc)) => Ok(vec![loc]),
            Some(GotoDefinitionResponse::Array(locs)) => Ok(locs),
            Some(GotoDefinitionResponse::Link(links)) => Ok(links
                .into_iter()
                .map(|link| Location {
                    uri: link.target_uri,
                    range: link.target_selection_range,
                })
                .collect()),
            None => Ok(vec![]),
        }
    }

    /// Rename a symbol across the workspace.
    pub async fn rename(
        &self,
        file_path: &Path,
        line: u32,
        character: u32,
        new_name: &str,
    ) -> Result<Option<WorkspaceEdit>> {
        if !self.is_ready().await {
            bail!("LSP server not initialized");
        }

        let params = RenameParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: Uri::from_str(&format!("file://{}", file_path.display()))
                        .map_err(|e| anyhow::anyhow!("Invalid file path: {}", e))?,
                },
                position: Position { line, character },
            },
            new_name: new_name.to_string(),
            work_done_progress_params: WorkDoneProgressParams::default(),
        };

        self.send_request("textDocument/rename", params).await
    }

    /// Expand macro at position (rust-analyzer specific).
    pub async fn expand_macro(
        &self,
        file_path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Option<String>> {
        if !self.is_ready().await {
            bail!("LSP server not initialized");
        }

        #[derive(Serialize)]
        struct ExpandMacroParams {
            #[serde(rename = "textDocument")]
            text_document: TextDocumentIdentifier,
            position: Position,
        }

        #[derive(Deserialize)]
        struct ExpandMacroResult {
            name: String,
            expansion: String,
        }

        let params = ExpandMacroParams {
            text_document: TextDocumentIdentifier {
                uri: Uri::from_str(&format!("file://{}", file_path.display()))
                    .map_err(|e| anyhow::anyhow!("Invalid file path: {}", e))?,
            },
            position: Position { line, character },
        };

        let response: Option<ExpandMacroResult> = self
            .send_request("rust-analyzer/expandMacro", params)
            .await?;

        Ok(response.map(|r| format!("// Macro: {}\n{}", r.name, r.expansion)))
    }

    /// Prepare call hierarchy for a position.
    pub async fn prepare_call_hierarchy(
        &self,
        file_path: &Path,
        line: u32,
        character: u32,
    ) -> Result<Vec<CallHierarchyItem>> {
        if !self.is_ready().await {
            bail!("LSP server not initialized");
        }

        let params = CallHierarchyPrepareParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: Uri::from_str(&format!("file://{}", file_path.display()))
                        .map_err(|e| anyhow::anyhow!("Invalid file path: {}", e))?,
                },
                position: Position { line, character },
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
        };

        let response: Option<Vec<CallHierarchyItem>> = self
            .send_request("textDocument/prepareCallHierarchy", params)
            .await?;

        Ok(response.unwrap_or_default())
    }

    /// Get incoming calls (callers) for a call hierarchy item.
    pub async fn incoming_calls(
        &self,
        item: CallHierarchyItem,
    ) -> Result<Vec<CallHierarchyIncomingCall>> {
        if !self.is_ready().await {
            bail!("LSP server not initialized");
        }

        let params = CallHierarchyIncomingCallsParams {
            item,
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };

        let response: Option<Vec<CallHierarchyIncomingCall>> = self
            .send_request("callHierarchy/incomingCalls", params)
            .await?;

        Ok(response.unwrap_or_default())
    }

    /// Get outgoing calls (callees) for a call hierarchy item.
    pub async fn outgoing_calls(
        &self,
        item: CallHierarchyItem,
    ) -> Result<Vec<CallHierarchyOutgoingCall>> {
        if !self.is_ready().await {
            bail!("LSP server not initialized");
        }

        let params = CallHierarchyOutgoingCallsParams {
            item,
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        };

        let response: Option<Vec<CallHierarchyOutgoingCall>> = self
            .send_request("callHierarchy/outgoingCalls", params)
            .await?;

        Ok(response.unwrap_or_default())
    }
}

impl Drop for GenericLspClient {
    fn drop(&mut self) {
        // Ensure process is killed when dropped
        if let Some(mut child) = self.process.try_lock().ok().and_then(|mut g| g.take()) {
            let _ = child.start_kill();
        }
    }
}
