# Changelog

All notable changes to this project will be documented in this file.

## [0.1.4] - 2026-04-25
### Fixed
- Resolved a hang in the `--setup` command by ensuring immediate process exit.
- Improved TOML configuration logic to robustly handle existing `mcp.servers` keys of different types.
### Added
- Comprehensive Agent Prompt in README.md for automated self-configuration.

## [0.1.3] - 2026-04-25
### Added
- Persistent rollback functionality with high-precision (nanosecond) backups.
- New `rollback` tool to undo file and configuration changes.
- Advanced editing tools: `append_to_file` and `replace_in_file`.
- New `--setup` CLI command for automatic ZeroClaw configuration.

## [0.1.2] - 2026-04-24
### Added
- Recursive file listing using `walkdir`.
- Automated parent directory creation for file writes.
- Proper `tracing` logging to `stderr`.
- Comprehensive documentation and Home Assistant add-on metadata.
- "Buy Me a Coffee" and "Add to Home Assistant" integration.
- CI/CD workflow for code quality.

## [0.1.1] - 2026-04-23
### Added
- Initial Rust implementation of the ZeroClaw Coordinator MCP.
- Substring-based blacklist validation.
- Support for stdio and SSE transports.
