#!/usr/bin/with-contenv bashio

export ZEROCLAW_WORKSPACE=$(bashio::config 'workspace_path')
export ZEROCLAW_BLACKLIST=$(bashio::config 'blacklist')
export ZEROCLAW_WHITELIST=$(bashio::config 'whitelist')
echo "Starting ZeroClaw Coordinator MCP Server..."
echo "Workspace: $ZEROCLAW_WORKSPACE"
echo "Whitelist: $ZEROCLAW_WHITELIST"
echo "Blacklist: $ZEROCLAW_BLACKLIST"

# Start the binary with SSE transport enabled on fixed container port 8090
/usr/local/bin/zeroclaw-coordinator-mcp --transport sse --port 8090
