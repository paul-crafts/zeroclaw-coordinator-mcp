#!/usr/bin/with-contenv bashio

export ZEROCLAW_WORKSPACE=$(bashio::config 'workspace_path')
export ZEROCLAW_BLACKLIST=$(bashio::config 'blacklist')
export ZEROCLAW_WHITELIST=$(bashio::config 'whitelist')
export PORT=$(bashio::config 'port')

echo "Starting ZeroClaw Coordinator MCP Server..."
echo "Workspace: $ZEROCLAW_WORKSPACE"
echo "Whitelist: $ZEROCLAW_WHITELIST"
echo "Blacklist: $ZEROCLAW_BLACKLIST"
echo "Port: $PORT"

# Start the binary with SSE transport enabled on the user-configured port
/usr/local/bin/zeroclaw-coordinator-mcp --transport sse --port $PORT
