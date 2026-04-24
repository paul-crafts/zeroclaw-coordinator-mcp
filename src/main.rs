mod toml_utils;

use anyhow::{Context, Result, anyhow};
use axum::{
    Json, Router,
    extract::State,
    response::sse::{Event, Sse},
    routing::{get, post},
};
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{self, AsyncBufReadExt, BufReader};
use toml_edit::DocumentMut;
use tower_http::cors::CorsLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

struct Server {
    workspace: PathBuf,
    blacklist: HashSet<String>,
    whitelist: Vec<PathBuf>,
}

impl Server {
    fn new(workspace: PathBuf, blacklist: Vec<String>, whitelist: Vec<String>) -> Self {
        let mut bl = HashSet::from_iter(blacklist);
        bl.insert("zeroclaw-coordinator-mcp".to_string());

        let wl = whitelist
            .iter()
            .map(|s| PathBuf::from(s.trim()))
            .filter(|p| !p.as_os_str().is_empty())
            .collect();

        Self {
            workspace,
            blacklist: bl,
            whitelist: wl,
        }
    }

    fn validate_path(&self, path: &Path) -> Result<()> {
        let abs_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.workspace.join(path)
        };

        // Check if the file name itself is blacklisted (e.g. IDENTITY.md)
        // Note: This uses a substring match, so blacklisting "IDENTITY" blocks "my_IDENTITY_file.txt"
        #[allow(clippy::collapsible_if)]
        if let Some(file_name) = abs_path.file_name().and_then(|n| n.to_str()) {
            if self.blacklist.iter().any(|b| file_name.contains(b)) {
                return Err(anyhow!("File '{}' is blacklisted", file_name));
            }
        }

        // Check if the path is within any whitelisted directory
        let is_whitelisted = self.whitelist.iter().any(|root| abs_path.starts_with(root));

        // Also allow access to the immediate ZEROCLAW_WORKSPACE for backward compatibility
        // (though ZEROCLAW_WORKSPACE should usually be in the whitelist anyway)
        let in_workspace = abs_path.starts_with(&self.workspace);

        if !is_whitelisted && !in_workspace {
            return Err(anyhow!(
                "Access denied: Path '{:?}' is not in the whitelist",
                abs_path
            ));
        }

        Ok(())
    }

    async fn handle_request(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        match req.method.as_str() {
            "initialize" => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: req.id,
                result: Some(json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "zeroclaw-coordinator", "version": "0.1.3" }
                })),
                error: None,
            },
            "tools/list" => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                id: req.id,
                result: Some(json!({
                    "tools": [
                        {
                            "name": "list_files",
                            "description": "List all files in the ZeroClaw workspace",
                            "inputSchema": { "type": "object", "properties": {} }
                        },
                        {
                            "name": "read_file",
                            "description": "Read the contents of a file in the workspace",
                            "inputSchema": {
                                "type": "object",
                                "properties": { "path": { "type": "string" } },
                                "required": ["path"]
                            }
                        },
                        {
                            "name": "write_file",
                            "description": "Write contents to a file in the workspace",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "path": { "type": "string" },
                                    "content": { "type": "string" }
                                },
                                "required": ["path", "content"]
                            }
                        },
                        {
                            "name": "set_config_value",
                            "description": "Set a specific value in config.toml using a path (e.g., 'agents.coder.model')",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "path": { "type": "string" },
                                    "value": { "type": "string", "description": "TOML value as string, e.g., '\"gpt-4\"' or 'true'" }
                                },
                                "required": ["path", "value"]
                            }
                        }
                    ]
                })),
                error: None,
            },
            "tools/call" => {
                let params = req.params.clone().unwrap_or(Value::Null);
                let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let args = params.get("arguments").unwrap_or(&Value::Null);

                match name {
                    "list_files" => {
                        if let Err(e) = self.validate_path(&self.workspace) {
                            return self.error(req.id, -32001, e.to_string());
                        }
                        match self.list_files() {
                            Ok(files) => self.success(req.id, json!({ "content": [{ "type": "text", "text": format!("{:?}", files) }] })),
                            Err(e) => self.error(req.id, -32000, e.to_string()),
                        }
                    }
                    "read_file" => {
                        let path_str = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                        if let Err(e) = self.validate_path(Path::new(path_str)) {
                            return self.error(req.id, -32001, e.to_string());
                        }
                        match self.read_file(path_str) {
                            Ok(content) => self.success(
                                req.id,
                                json!({ "content": [{ "type": "text", "text": content }] }),
                            ),
                            Err(e) => self.error(req.id, -32000, e.to_string()),
                        }
                    }
                    "write_file" => {
                        let path_str = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                        let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
                        if let Err(e) = self.validate_path(Path::new(path_str)) {
                            return self.error(req.id, -32001, e.to_string());
                        }
                        match self.write_file(path_str, content) {
                            Ok(_) => self.success(req.id, json!({ "content": [{ "type": "text", "text": "File written successfully" }] })),
                            Err(e) => self.error(req.id, -32000, e.to_string()),
                        }
                    }
                    "set_config_value" => {
                        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                        let value = args.get("value").and_then(|v| v.as_str()).unwrap_or("");
                        if let Err(e) = self.validate_path(&PathBuf::from("config.toml")) {
                            return self.error(req.id, -32001, e.to_string());
                        }
                        match self.set_config_value(path, value) {
                            Ok(_) => self.success(req.id, json!({ "content": [{ "type": "text", "text": "Config updated successfully" }] })),
                            Err(e) => self.error(req.id, -32000, e.to_string()),
                        }
                    }
                    _ => self.error(req.id, -32601, "Method not found".to_string()),
                }
            }
            _ => self.error(req.id, -32601, "Method not found".to_string()),
        }
    }

    fn success(&self, id: Option<Value>, result: Value) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(&self, id: Option<Value>, code: i32, message: String) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError { code, message }),
        }
    }

    fn list_files(&self) -> Result<Vec<String>> {
        let mut files = Vec::new();
        for entry in walkdir::WalkDir::new(&self.workspace) {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            #[allow(clippy::collapsible_if)]
            if entry.file_type().is_file() {
                if let Ok(rel_path) = entry.path().strip_prefix(&self.workspace) {
                    if let Some(name) = rel_path.to_str() {
                        if self.validate_path(entry.path()).is_ok() {
                            files.push(name.to_string());
                        }
                    }
                }
            }
        }
        Ok(files)
    }

    fn read_file(&self, rel_path: &str) -> Result<String> {
        let path = if Path::new(rel_path).is_absolute() {
            PathBuf::from(rel_path)
        } else {
            self.workspace.join(rel_path)
        };
        fs::read_to_string(path).context("Failed to read file")
    }

    fn write_file(&self, rel_path: &str, content: &str) -> Result<()> {
        let path = if Path::new(rel_path).is_absolute() {
            PathBuf::from(rel_path)
        } else {
            self.workspace.join(rel_path)
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create parent directories")?;
        }
        fs::write(path, content).context("Failed to write file")
    }

    fn set_config_value(&self, path: &str, value: &str) -> Result<()> {
        let config_path = self.workspace.join("config.toml");
        let content = fs::read_to_string(&config_path).context("Failed to read config.toml")?;
        let mut doc: DocumentMut = content.parse().context("Failed to parse config.toml")?;
        toml_utils::set_value_by_path(&mut doc, path, value)?;
        fs::write(config_path, doc.to_string()).context("Failed to save config.toml")
    }
}

struct AppState {
    server: Arc<Server>,
    tx: Arc<tokio::sync::broadcast::Sender<JsonRpcResponse>>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();

    let args: Vec<String> = env::args().collect();
    let transport = if let Some(i) = args.iter().position(|a| a == "--transport") {
        if let Some(s) = args.get(i + 1) {
            s.as_str()
        } else {
            "stdio"
        }
    } else {
        "stdio"
    };

    let port = if let Some(i) = args.iter().position(|a| a == "--port") {
        if let Some(s) = args.get(i + 1) {
            s.parse::<u16>().unwrap_or(8090)
        } else {
            8090
        }
    } else {
        8090
    };

    let workspace = env::var("ZEROCLAW_WORKSPACE").unwrap_or_else(|_| ".".to_string());
    let blacklist_str = env::var("ZEROCLAW_BLACKLIST").unwrap_or_default();
    let blacklist: Vec<String> = blacklist_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let whitelist_str = env::var("ZEROCLAW_WHITELIST").unwrap_or_default();
    let whitelist: Vec<String> = whitelist_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let server = Arc::new(Server::new(PathBuf::from(workspace), blacklist, whitelist));

    if transport == "sse" {
        let (tx, _) = tokio::sync::broadcast::channel(100);
        let app_state = Arc::new(AppState {
            server: server.clone(),
            tx: Arc::new(tx),
        });

        let app = Router::new()
            .route("/sse", get(sse_handler))
            .route("/messages", post(message_handler))
            .layer(CorsLayer::permissive())
            .with_state(app_state);

        tracing::info!("Starting SSE server on port {}...", port);
        let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port)).await?;
        axum::serve(listener, app).await?;
    } else {
        let mut lines = BufReader::new(io::stdin()).lines();
        while let Some(line) = lines.next_line().await? {
            if let Ok(req) = serde_json::from_str::<JsonRpcRequest>(&line) {
                let resp = server.handle_request(req).await;
                println!("{}", serde_json::to_string(&resp)?);
            }
        }
    }

    Ok(())
}

async fn sse_handler(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, io::Error>>> {
    let mut rx = state.tx.subscribe();
    let endpoint_event = Event::default().event("endpoint").data("/messages");
    let stream = async_stream::stream! {
        yield Ok(endpoint_event);
        while let Ok(msg) = rx.recv().await {
            let json = serde_json::to_string(&msg).unwrap();
            yield Ok(Event::default().data(json));
        }
    };
    Sse::new(stream)
}

async fn message_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<JsonRpcRequest>,
) -> Json<JsonRpcResponse> {
    let resp = state.server.handle_request(req).await;
    let _ = state.tx.send(resp.clone());
    Json(resp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_whitelist_validation() {
        let server = Server::new(
            PathBuf::from("/zeroclaw"),
            vec![],
            vec![
                "/config/zeroclaw".to_string(),
                "/share/zeroclaw".to_string(),
            ],
        );

        // Allowed paths
        assert!(
            server
                .validate_path(Path::new("/config/zeroclaw/config.toml"))
                .is_ok()
        );
        assert!(
            server
                .validate_path(Path::new("/share/zeroclaw/data.json"))
                .is_ok()
        );

        // Denied paths
        assert!(
            server
                .validate_path(Path::new("/config/secrets.yaml"))
                .is_err()
        );
        assert!(server.validate_path(Path::new("/etc/passwd")).is_err());
    }

    #[test]
    fn test_blacklist_validation() {
        let server = Server::new(
            PathBuf::from("/zeroclaw"),
            vec!["IDENTITY.md".to_string()],
            vec!["/zeroclaw".to_string()],
        );

        // Blacklisted file in whitelisted folder
        assert!(
            server
                .validate_path(Path::new("/zeroclaw/IDENTITY.md"))
                .is_err()
        );

        // Regular file in whitelisted folder
        assert!(
            server
                .validate_path(Path::new("/zeroclaw/config.toml"))
                .is_ok()
        );
    }

    #[test]
    fn test_workspace_fallback_validation() {
        let server = Server::new(
            PathBuf::from("/custom_ws"),
            vec![],
            vec![], // Empty whitelist
        );

        // Should allow access to its own workspace even if whitelist is empty
        assert!(
            server
                .validate_path(Path::new("/custom_ws/file.txt"))
                .is_ok()
        );
    }

    #[test]
    fn test_blacklist_substring_validation() {
        let server = Server::new(
            PathBuf::from("/zeroclaw"),
            vec!["IDENTITY".to_string()],
            vec!["/zeroclaw".to_string()],
        );

        // Exact match
        assert!(
            server
                .validate_path(Path::new("/zeroclaw/IDENTITY"))
                .is_err()
        );

        // Substring match
        assert!(
            server
                .validate_path(Path::new("/zeroclaw/my_IDENTITY_file.txt"))
                .is_err()
        );
        assert!(
            server
                .validate_path(Path::new("/zeroclaw/IDENTITY.md"))
                .is_err()
        );

        // Allowed
        assert!(
            server
                .validate_path(Path::new("/zeroclaw/ident.txt"))
                .is_ok()
        );
    }

    #[test]
    fn test_validate_relative_paths() {
        let server = Server::new(
            PathBuf::from("/zeroclaw"),
            vec![],
            vec!["/zeroclaw".to_string()],
        );

        // Relative paths should resolve to workspace and be allowed
        assert!(server.validate_path(Path::new("config.toml")).is_ok());
        assert!(server.validate_path(Path::new("nested/file.txt")).is_ok());
    }

    #[test]
    fn test_write_and_list_files_recursive() {
        let temp_dir = tempfile::tempdir().unwrap();
        let workspace = temp_dir.path().to_path_buf();
        let server = Server::new(
            workspace.clone(),
            vec![],
            vec![workspace.to_string_lossy().to_string()],
        );

        // Test writing a file to a nested non-existent directory
        let rel_path = "nested/dir/file.txt";
        let content = "hello world";
        assert!(server.write_file(rel_path, content).is_ok());

        // Verify the file was created and content is correct
        let abs_path = workspace.join(rel_path);
        assert!(abs_path.exists());
        assert_eq!(std::fs::read_to_string(&abs_path).unwrap(), content);

        // Test recursive file listing
        let files = server.list_files().unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("file.txt"));
    }
}
