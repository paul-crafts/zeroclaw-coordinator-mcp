# Documentation

## ⚙️ Configuration

### Environment Variables
When running the standalone binary via `stdio`, configure the server using environment variables:

- `ZEROCLAW_WORKSPACE`: The primary workspace directory (default: `.`)
- `ZEROCLAW_WHITELIST`: Comma-separated list of allowed paths.
- `ZEROCLAW_BLACKLIST`: Comma-separated list of substrings to block (e.g., `IDENTITY.md,secrets`).
- `ZEROCLAW_PORT`: The port used for the SSE transport (if enabled, default: 8090).

For Home Assistant Add-on users, these are configured via the **Configuration** tab in the UI.

The ZeroClaw Coordinator MCP server can be configured via the Home Assistant Add-on options page.

### Options

| Option | Type | Default | Description |
| :--- | :--- | :--- | :--- |
| `workspace_path` | string | `/config` | The primary directory where ZeroClaw configuration files are stored. |
| `blacklist` | string | `IDENTITY.md` | A comma-separated list of filenames or substrings to block from access. |
| `whitelist` | string | `/config,/share` | A comma-separated list of directories that are allowed to be accessed. |
| `port` | integer | `8090` | The port used for the SSE transport (if enabled). |

### Environment Variables (for Standalone/Binary)

When running the binary directly, use these environment variables:

| Variable | Description |
| :--- | :--- |
| `ZEROCLAW_WORKSPACE` | Path to the workspace (defaults to current directory). |
| `ZEROCLAW_WHITELIST` | Comma-separated list of allowed directory paths. |
| `ZEROCLAW_BLACKLIST` | Comma-separated list of substrings to block. |
| `RUST_LOG` | Logging level (`info`, `debug`, `error`). |


## Usage

This add-on exposes several tools via the Model Context Protocol (MCP):

1. **list_files**: Recursively lists all files in the configured workspace.
2. **read_file**: Reads the content of a specific file.
3. **write_file**: Writes content to a file, automatically creating parent directories if needed.
4. **set_config_value**: Specifically updates values in `config.toml`.

## Security

The add-on implements strict path validation. Access is only granted to files that:
- Are within the `workspace_path` or the `whitelist` directories.
- Do not contain any of the substrings defined in the `blacklist`.
