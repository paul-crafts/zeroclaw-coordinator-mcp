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
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
enum ChangeType {
    Update,
    Create,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HistoryEntry {
    timestamp: u64,
    path: PathBuf,
    change_type: ChangeType,
    backup_file: Option<String>, // Filename in history directory
}

struct HistoryManager {
    history_dir: PathBuf,
    limit: usize,
}

impl HistoryManager {
    fn new(history_dir: PathBuf, limit: usize) -> Self {
        if !history_dir.exists() {
            let _ = fs::create_dir_all(&history_dir);
        }
        Self { history_dir, limit }
    }

    fn get_manifest_path(&self) -> PathBuf {
        self.history_dir.join("history.json")
    }

    fn load_history(&self) -> Vec<HistoryEntry> {
        let path = self.get_manifest_path();
        if let Ok(content) = fs::read_to_string(path) {
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    fn save_history(&self, history: &[HistoryEntry]) -> Result<()> {
        let path = self.get_manifest_path();
        let content = serde_json::to_string_pretty(history)?;
        fs::write(path, content).context("Failed to save history manifest")
    }

    fn add_entry(&self, entry: HistoryEntry) -> Result<()> {
        let mut history = self.load_history();
        history.push(entry);

        // Enforce limit
        while history.len() > self.limit {
            let removed = history.remove(0);
            if let Some(backup_file) = removed.backup_file {
                let _ = fs::remove_file(self.history_dir.join(backup_file));
            }
        }

        self.save_history(&history)
    }

    fn pop_entry(&self) -> Option<HistoryEntry> {
        let mut history = self.load_history();
        let entry = history.pop()?;
        let _ = self.save_history(&history);
        Some(entry)
    }
}

struct Server {
    workspace: PathBuf,
    blacklist: HashSet<String>,
    whitelist: Vec<PathBuf>,
    history: HistoryManager,
}

impl Server {
    fn new(
        workspace: PathBuf,
        blacklist: Vec<String>,
        whitelist: Vec<String>,
        history_dir: PathBuf,
    ) -> Self {
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
            history: HistoryManager::new(history_dir, 10),
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
                    "serverInfo": { "name": "zeroclaw-coordinator", "version": "0.1.4" }
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
                        },
                        {
                            "name": "rollback",
                            "description": "Rollback the last change made by the server",
                            "inputSchema": { "type": "object", "properties": {} }
                        },
                        {
                            "name": "append_to_file",
                            "description": "Append content to a file in the workspace",
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
                            "name": "replace_in_file",
                            "description": "Replace text in a file in the workspace",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "path": { "type": "string" },
                                    "target": { "type": "string", "description": "The text to be replaced" },
                                    "replacement": { "type": "string", "description": "The text to replace with" }
                                },
                                "required": ["path", "target", "replacement"]
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
                    "rollback" => match self.rollback() {
                        Ok(msg) => self.success(
                            req.id,
                            json!({ "content": [{ "type": "text", "text": msg }] }),
                        ),
                        Err(e) => self.error(req.id, -32000, e.to_string()),
                    },
                    "append_to_file" => {
                        let path_str = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                        let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
                        if let Err(e) = self.validate_path(Path::new(path_str)) {
                            return self.error(req.id, -32001, e.to_string());
                        }
                        match self.append_to_file(path_str, content) {
                            Ok(_) => self.success(req.id, json!({ "content": [{ "type": "text", "text": "Content appended successfully" }] })),
                            Err(e) => self.error(req.id, -32000, e.to_string()),
                        }
                    }
                    "replace_in_file" => {
                        let path_str = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
                        let target = args.get("target").and_then(|v| v.as_str()).unwrap_or("");
                        let replacement = args
                            .get("replacement")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if let Err(e) = self.validate_path(Path::new(path_str)) {
                            return self.error(req.id, -32001, e.to_string());
                        }
                        match self.replace_in_file(path_str, target, replacement) {
                            Ok(_) => self.success(req.id, json!({ "content": [{ "type": "text", "text": "Text replaced successfully" }] })),
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
        let mut unique_roots = HashSet::new();
        unique_roots.insert(&self.workspace);
        for root in &self.whitelist {
            unique_roots.insert(root);
        }

        for root in unique_roots {
            for entry in walkdir::WalkDir::new(root) {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                #[allow(clippy::collapsible_if)]
                if entry.file_type().is_file() {
                    if let Some(path_str) = entry.path().to_str() {
                        if self.validate_path(entry.path()).is_ok() {
                            files.push(path_str.to_string());
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
        self.validate_path(&path)?;
        fs::read_to_string(path).context("Failed to read file")
    }

    fn write_file(&self, rel_path: &str, content: &str) -> Result<()> {
        let path = if Path::new(rel_path).is_absolute() {
            PathBuf::from(rel_path)
        } else {
            self.workspace.join(rel_path)
        };
        self.validate_path(&path)?;
        self.record_change(&path)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create parent directories")?;
        }
        fs::write(path, content).context("Failed to write file")
    }

    fn set_config_value(&self, path: &str, value: &str) -> Result<()> {
        let config_path = self.workspace.join("config.toml");
        self.validate_path(&config_path)?;
        self.record_change(&config_path)?;
        let content = fs::read_to_string(&config_path).context("Failed to read config.toml")?;
        let mut doc: DocumentMut = content.parse().context("Failed to parse config.toml")?;
        toml_utils::set_value_by_path(&mut doc, path, value)?;
        fs::write(config_path, doc.to_string()).context("Failed to save config.toml")
    }

    fn append_to_file(&self, rel_path: &str, content: &str) -> Result<()> {
        let path = if Path::new(rel_path).is_absolute() {
            PathBuf::from(rel_path)
        } else {
            self.workspace.join(rel_path)
        };
        self.validate_path(&path)?;
        self.record_change(&path)?;
        let mut existing_content = if path.exists() {
            fs::read_to_string(&path)?
        } else {
            String::new()
        };
        if !existing_content.is_empty() && !existing_content.ends_with('\n') {
            existing_content.push('\n');
        }
        existing_content.push_str(content);
        if !content.ends_with('\n') {
            existing_content.push('\n');
        }
        fs::write(path, existing_content).context("Failed to append to file")
    }

    fn replace_in_file(&self, rel_path: &str, target: &str, replacement: &str) -> Result<()> {
        let path = if Path::new(rel_path).is_absolute() {
            PathBuf::from(rel_path)
        } else {
            self.workspace.join(rel_path)
        };
        self.validate_path(&path)?;
        self.record_change(&path)?;
        let content = fs::read_to_string(&path).context("Failed to read file for replacement")?;
        let new_content = content.replace(target, replacement);
        fs::write(path, new_content).context("Failed to write file after replacement")
    }

    fn record_change(&self, path: &Path) -> Result<()> {
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_nanos();
        if path.exists() {
            let old_content = fs::read_to_string(path)?;
            let filename = path
                .file_name()
                .ok_or_else(|| anyhow!("No filename"))?
                .to_string_lossy();
            let backup_filename = format!("{}_{}", timestamp, filename);
            fs::write(self.history.history_dir.join(&backup_filename), old_content)?;
            self.history.add_entry(HistoryEntry {
                timestamp: (timestamp / 1_000_000_000) as u64,
                path: path.to_path_buf(),
                change_type: ChangeType::Update,
                backup_file: Some(backup_filename),
            })?;
        } else {
            self.history.add_entry(HistoryEntry {
                timestamp: (timestamp / 1_000_000_000) as u64,
                path: path.to_path_buf(),
                change_type: ChangeType::Create,
                backup_file: None,
            })?;
        }
        Ok(())
    }

    fn rollback(&self) -> Result<String> {
        if let Some(entry) = self.history.pop_entry() {
            match entry.change_type {
                ChangeType::Update => {
                    let backup_file = entry
                        .backup_file
                        .ok_or_else(|| anyhow!("Missing backup file"))?;
                    let backup_path = self.history.history_dir.join(&backup_file);
                    let content = fs::read_to_string(&backup_path)?;
                    fs::write(&entry.path, content)?;
                    let _ = fs::remove_file(backup_path);
                    Ok(format!("Rolled back update to {:?}", entry.path))
                }
                ChangeType::Create => {
                    if entry.path.exists() {
                        fs::remove_file(&entry.path)?;
                    }
                    Ok(format!("Rolled back creation of {:?}", entry.path))
                }
            }
        } else {
            Err(anyhow!("No history to rollback"))
        }
    }
}

fn configure_mcp_server(config_path: &Path, exe_path: &Path) -> Result<()> {
    let exe_str = exe_path.to_string_lossy().to_string();
    let content = fs::read_to_string(config_path)?;
    let mut doc: DocumentMut = content.parse().context("Failed to parse config.toml")?;

    // 1. Ensure [mcp] exists and is a regular Table
    if let Some(mcp) = doc.get_mut("mcp") {
        if !mcp.is_table() {
            *mcp = toml_edit::Item::Table(toml_edit::Table::new());
        }
    } else {
        doc.insert("mcp", toml_edit::Item::Table(toml_edit::Table::new()));
    }

    let mcp = doc.get_mut("mcp").unwrap().as_table_mut().unwrap();
    mcp.insert("enabled", toml_edit::value(true));

    // 2. Ensure mcp.servers exists and is an ArrayOfTables
    if let Some(servers) = mcp.get_mut("servers") {
        if servers.as_array_of_tables().is_none() {
            *servers = toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
        }
    } else {
        mcp.insert(
            "servers",
            toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new()),
        );
    }

    let servers = mcp
        .get_mut("servers")
        .unwrap()
        .as_array_of_tables_mut()
        .unwrap();

    // 3. Find or Add the coordinator server
    let mut found = false;
    for server in servers.iter_mut() {
        if server.get("name").and_then(|n| n.as_str()) == Some("coordinator") {
            server["command"] = toml_edit::value(&exe_str);
            let mut args = toml_edit::Array::new();
            args.push("--transport");
            args.push("stdio");
            server["args"] = toml_edit::Item::Value(toml_edit::Value::Array(args));
            found = true;
            break;
        }
    }

    if !found {
        let mut new_server = toml_edit::Table::new();
        new_server.insert("name", toml_edit::value("coordinator"));
        new_server.insert("transport", toml_edit::value("stdio"));
        new_server.insert("command", toml_edit::value(&exe_str));
        let mut args = toml_edit::Array::new();
        args.push("--transport");
        args.push("stdio");
        new_server.insert(
            "args",
            toml_edit::Item::Value(toml_edit::Value::Array(args)),
        );
        servers.push(new_server);
    }

    fs::write(config_path, doc.to_string())?;
    Ok(())
}

fn run_setup() -> Result<()> {
    let home = env::var("HOME").context("Failed to get HOME environment variable")?;
    let config_dir = Path::new(&home).join(".zeroclaw");
    let config_path = config_dir.join("config.toml");

    if !config_path.exists() {
        return Err(anyhow!(
            "ZeroClaw config not found at {}. Please run 'zeroclaw onboard' first.",
            config_path.display()
        ));
    }

    let exe_path = env::current_exe()?.canonicalize()?;
    configure_mcp_server(&config_path, &exe_path)?;

    println!(
        "Successfully configured coordinator MCP in {}",
        config_path.display()
    );
    Ok(())
}

struct AppState {
    server: Arc<Server>,
    tx: Arc<tokio::sync::broadcast::Sender<JsonRpcResponse>>,
}

fn get_config() -> (String, String, String, u16, String) {
    let args: Vec<String> = env::args().collect();
    let transport = if let Some(i) = args.iter().position(|a| a == "--transport") {
        if let Some(s) = args.get(i + 1) {
            s.clone()
        } else {
            "stdio".to_string()
        }
    } else {
        "stdio".to_string()
    };

    let port = if let Some(i) = args.iter().position(|a| a == "--port") {
        if let Some(s) = args.get(i + 1) {
            s.parse::<u16>().unwrap_or(8090)
        } else {
            8090
        }
    } else {
        env::var("ZEROCLAW_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8090)
    };

    let workspace = env::var("ZEROCLAW_WORKSPACE").unwrap_or_else(|_| ".".to_string());
    let blacklist_str = env::var("ZEROCLAW_BLACKLIST").unwrap_or_default();
    let whitelist_str = env::var("ZEROCLAW_WHITELIST").unwrap_or_default();

    (workspace, blacklist_str, whitelist_str, port, transport)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    if args.iter().any(|a| a == "--setup") {
        run_setup()?;
        std::process::exit(0);
    }

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "zeroclaw_coordinator_mcp=info,tower_http=debug,axum::rejection=trace".into()
            }),
        )
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();

    let (workspace, blacklist_str, whitelist_str, port, transport) = get_config();

    let blacklist: Vec<String> = blacklist_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let whitelist: Vec<String> = whitelist_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let history_dir = env::var("HOME")
        .map(|h| PathBuf::from(h).join(".zeroclaw/mcp-coordinator/history"))
        .unwrap_or_else(|_| PathBuf::from(".zeroclaw_history"));

    let server = Arc::new(Server::new(
        PathBuf::from(workspace),
        blacklist,
        whitelist,
        history_dir,
    ));

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

        let addr = format!("0.0.0.0:{}", port);
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        tracing::info!("SSE server listening on {}", addr);
        axum::serve(listener, app).await?;
    } else {
        let stdin = io::stdin();
        let mut reader = BufReader::new(stdin).lines();

        while let Some(line) = reader.next_line().await? {
            let req: JsonRpcRequest = match serde_json::from_str(&line) {
                Ok(r) => r,
                Err(_) => continue,
            };

            let resp = server.handle_request(req).await;
            println!("{}", serde_json::to_string(&resp)?);
        }
    }

    Ok(())
}

async fn sse_handler(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.tx.subscribe();
    let stream = async_stream::stream! {
        while let Ok(msg) = rx.recv().await {
            yield Ok(Event::default().data(serde_json::to_string(&msg).unwrap()));
        }
    };

    Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default())
}

use std::convert::Infallible;

async fn message_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<JsonRpcRequest>,
) -> Json<JsonRpcResponse> {
    let resp = state.server.handle_request(req).await;
    Json(resp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_path_validation() {
        let temp_history = tempfile::tempdir().unwrap();
        let server = Server::new(
            PathBuf::from("/zeroclaw"),
            vec!["IDENTITY.md".to_string()],
            vec!["/zeroclaw".to_string(), "/share/zeroclaw".to_string()],
            temp_history.path().to_path_buf(),
        );

        // Allowed
        assert!(
            server
                .validate_path(Path::new("/zeroclaw/config.toml"))
                .is_ok()
        );
        assert!(
            server
                .validate_path(Path::new("/share/zeroclaw/data.json"))
                .is_ok()
        );

        // Denied (outside whitelist)
        assert!(server.validate_path(Path::new("/etc/passwd")).is_err());

        // Denied (blacklisted)
        assert!(
            server
                .validate_path(Path::new("/zeroclaw/IDENTITY.md"))
                .is_err()
        );
    }

    #[test]
    fn test_workspace_fallback_validation() {
        let temp_history = tempfile::tempdir().unwrap();
        let server = Server::new(
            PathBuf::from("/custom_ws"),
            vec![],
            vec![], // Empty whitelist
            temp_history.path().to_path_buf(),
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
        let temp_history = tempfile::tempdir().unwrap();
        let server = Server::new(
            PathBuf::from("/zeroclaw"),
            vec!["IDENTITY".to_string()],
            vec!["/zeroclaw".to_string()],
            temp_history.path().to_path_buf(),
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
        let temp_history = tempfile::tempdir().unwrap();
        let server = Server::new(
            PathBuf::from("/zeroclaw"),
            vec![],
            vec!["/zeroclaw".to_string()],
            temp_history.path().to_path_buf(),
        );

        // Relative paths should resolve to workspace and be allowed
        assert!(server.validate_path(Path::new("config.toml")).is_ok());
        assert!(server.validate_path(Path::new("nested/file.txt")).is_ok());
    }

    #[test]
    fn test_write_and_list_files_recursive() {
        let temp_dir = tempfile::tempdir().unwrap();
        let temp_history = tempfile::tempdir().unwrap();
        let workspace = temp_dir.path().to_path_buf();
        let server = Server::new(
            workspace.clone(),
            vec![],
            vec![workspace.to_string_lossy().to_string()],
            temp_history.path().to_path_buf(),
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

    #[test]
    fn test_config_from_env() {
        unsafe {
            std::env::set_var("ZEROCLAW_WORKSPACE", "/tmp/ws");
            std::env::set_var("ZEROCLAW_BLACKLIST", "secret,private");
            std::env::set_var("ZEROCLAW_WHITELIST", "/tmp/other,/var/log");
            std::env::set_var("ZEROCLAW_PORT", "9999");
        }

        let (workspace, blacklist, whitelist, port, transport) = get_config();

        assert_eq!(workspace, "/tmp/ws");
        assert_eq!(blacklist, "secret,private");
        assert_eq!(whitelist, "/tmp/other,/var/log");
        assert_eq!(port, 9999);
        assert_eq!(transport, "stdio"); // Default

        // Clean up
        unsafe {
            std::env::remove_var("ZEROCLAW_WORKSPACE");
            std::env::remove_var("ZEROCLAW_BLACKLIST");
            std::env::remove_var("ZEROCLAW_WHITELIST");
            std::env::remove_var("ZEROCLAW_PORT");
        }
    }

    #[test]
    fn test_complex_multi_workspace_scenario() {
        let tmp_ws1 = tempfile::tempdir().unwrap();
        let tmp_ws2 = tempfile::tempdir().unwrap();
        let temp_history = tempfile::tempdir().unwrap();

        let ws1_path = tmp_ws1.path().to_path_buf();
        let ws2_path = tmp_ws2.path().to_path_buf();

        let server = Server::new(
            ws1_path.clone(),
            vec!["private".to_string()],
            vec![
                ws1_path.to_string_lossy().to_string(),
                ws2_path.to_string_lossy().to_string(),
            ],
            temp_history.path().to_path_buf(),
        );

        // 1. Test writing to both workspaces
        assert!(
            server
                .write_file(&ws1_path.join("public.txt").to_string_lossy(), "data1")
                .is_ok()
        );
        assert!(
            server
                .write_file(&ws2_path.join("public.txt").to_string_lossy(), "data2")
                .is_ok()
        );

        // 2. Test blacklisting in both workspaces
        assert!(
            server
                .write_file(
                    &ws1_path.join("private_info.txt").to_string_lossy(),
                    "secret"
                )
                .is_err()
        );
        assert!(
            server
                .write_file(
                    &ws2_path.join("private_info.txt").to_string_lossy(),
                    "secret"
                )
                .is_err()
        );

        // 3. Test listing includes all workspaces
        let files = server.list_files().unwrap();
        assert!(files.iter().any(|f| f.contains("public.txt")));
        // Should contain 2 files (one in each ws)
        let public_files: Vec<_> = files.iter().filter(|f| f.contains("public.txt")).collect();
        assert_eq!(public_files.len(), 2);

        // 4. Test access outside both workspaces
        assert!(server.validate_path(Path::new("/etc/shadow")).is_err());
    }

    #[test]
    fn test_rollback_functionality() {
        let temp_dir = tempfile::tempdir().unwrap();
        let temp_history = tempfile::tempdir().unwrap();
        let workspace = temp_dir.path().to_path_buf();
        let server = Server::new(
            workspace.clone(),
            vec![],
            vec![workspace.to_string_lossy().to_string()],
            temp_history.path().to_path_buf(),
        );

        let file_path = "test.txt";
        let content1 = "initial content";
        let content2 = "updated content";

        // 1. Rollback creation
        server.write_file(file_path, content1).unwrap();
        assert!(workspace.join(file_path).exists());
        server.rollback().unwrap();
        assert!(!workspace.join(file_path).exists());

        // 2. Rollback update
        server.write_file(file_path, content1).unwrap();
        server.write_file(file_path, content2).unwrap();
        assert_eq!(
            fs::read_to_string(workspace.join(file_path)).unwrap(),
            content2
        );
        server.rollback().unwrap();
        assert_eq!(
            fs::read_to_string(workspace.join(file_path)).unwrap(),
            content1
        );

        // 3. Rollback config update
        let config_path = workspace.join("config.toml");
        fs::write(&config_path, "key = \"old\"").unwrap();
        server.set_config_value("key", "\"new\"").unwrap();
        assert!(
            fs::read_to_string(&config_path)
                .unwrap()
                .contains("\"new\"")
        );
        server.rollback().unwrap();
        assert!(
            fs::read_to_string(&config_path)
                .unwrap()
                .contains("\"old\"")
        );
    }

    #[test]
    fn test_advanced_editing_tools() {
        let temp_dir = tempfile::tempdir().unwrap();
        let temp_history = tempfile::tempdir().unwrap();
        let workspace = temp_dir.path().to_path_buf();
        let server = Server::new(
            workspace.clone(),
            vec![],
            vec![workspace.to_string_lossy().to_string()],
            temp_history.path().to_path_buf(),
        );

        let file_path = "edit_test.txt";

        // 1. Test append_to_file
        server.append_to_file(file_path, "line 1").unwrap();
        server.append_to_file(file_path, "line 2").unwrap();
        let content = fs::read_to_string(workspace.join(file_path)).unwrap();
        assert!(content.contains("line 1\nline 2\n"));

        // 2. Test replace_in_file
        server
            .replace_in_file(file_path, "line 1", "first line")
            .unwrap();
        let content = fs::read_to_string(workspace.join(file_path)).unwrap();
        assert!(content.contains("first line\nline 2\n"));

        // 3. Test rollback of advanced tools
        server.rollback().unwrap(); // undo replace
        let content = fs::read_to_string(workspace.join(file_path)).unwrap();
        assert!(content.contains("line 1\nline 2\n"));

        server.rollback().unwrap(); // undo second append
        let content = fs::read_to_string(workspace.join(file_path)).unwrap();
        assert_eq!(content, "line 1\n");
    }

    #[test]
    fn test_setup_command_logic() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config_path = temp_dir.path().join("config.toml");
        let exe_path = PathBuf::from("/usr/local/bin/coordinator");

        // 1. Initial config
        fs::write(&config_path, "default_provider = \"anthropic\"\n").unwrap();

        // 2. Run configuration
        configure_mcp_server(&config_path, &exe_path).unwrap();

        let content = fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("[mcp]"));
        assert!(content.contains("enabled = true"));
        assert!(content.contains("[[mcp.servers]]"));
        assert!(content.contains("name = \"coordinator\""));
        assert!(content.contains("command = \"/usr/local/bin/coordinator\""));

        // 3. Run again, should update (or stay same)
        let new_exe = PathBuf::from("/opt/bin/coordinator");
        configure_mcp_server(&config_path, &new_exe).unwrap();

        let content = fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("command = \"/opt/bin/coordinator\""));
        // Should still only have one coordinator
        let count = content.matches("name = \"coordinator\"").count();
        assert_eq!(count, 1);
    }
}
