# 🤖 ZeroClaw Coordinator MCP

[![GitHub Release](https://img.shields.io/github/v/release/paul-crafts/zeroclaw-coordinator-mcp?style=flat-square)](https://github.com/paul-crafts/zeroclaw-coordinator-mcp/releases)
[![Build Status](https://img.shields.io/github/actions/workflow/status/paul-crafts/zeroclaw-coordinator-mcp/ci.yml?branch=main&style=flat-square)](https://github.com/paul-crafts/zeroclaw-coordinator-mcp/actions)
[![License](https://img.shields.io/github/license/paul-crafts/zeroclaw-coordinator-mcp?style=flat-square)](LICENSE)
[![Buy Me A Coffee](https://img.shields.io/badge/buy%20me%20a%20coffee-donate-yellow.svg?style=flat-square)](https://www.buymeacoffee.com/paul.crafts)

![ZeroClaw Logo](logo.png)

The **ZeroClaw Coordinator MCP** is a high-performance Model Context Protocol (MCP) server built in Rust, designed specifically for Home Assistant environments. It enables AI agents to securely coordinate and manage your ZeroClaw configurations through a robust toolset.

---

## 🚀 Installation

### Add to Home Assistant
Click the button below to add this repository to your Home Assistant instance:

[![Open your Home Assistant instance and show the add-on store by clicking this button.](https://my.home-assistant.io/badges/supervisor_addon.svg)](https://my.home-assistant.io/redirect/supervisor_addon/?addon=zeroclaw_coordinator_mcp&repository_url=https%3A%2F%2Fgithub.com%2Fpaul-crafts%2Fzeroclaw-coordinator-mcp)

### Manual Installation
1. Navigate to your Home Assistant Settings -> Add-ons.
2. Click the **Add-on Store** button in the bottom right.
3. Click the vertical dots in the top right and select **Repositories**.
4. Add `https://github.com/paul-crafts/zeroclaw-coordinator-mcp` to the list.
5. Search for "ZeroClaw Coordinator MCP" and click **Install**.

### 📦 Standalone Binaries
For non-Home Assistant users or advanced setups, pre-compiled binaries are available for Linux, macOS, and Windows on the [Releases page](https://github.com/paul-crafts/zeroclaw-coordinator-mcp/releases).

---

## 🔗 Integration

### ZeroClaw Client Configuration
To enable an AI agent (like ZeroClaw) to use this coordinator, you need to add it to your MCP client configuration.

#### 🌐 SSE Transport (Recommended for Home Assistant)
Since Home Assistant add-ons run in isolated containers, **SSE is the easiest way** for them to communicate. Use the internal add-on hostname:

```json
{
  "mcpServers": {
    "zeroclaw-coordinator": {
      "url": "http://addon_zeroclaw_coordinator_mcp:8090/sse"
    }
  }
}
```

#### 🐚 Stdio Transport (Local/Standalone)
Use `stdio` only if the client and server are running on the same host (e.g., during local development or in a non-containerized setup).

```json
{
  "mcpServers": {
    "zeroclaw-coordinator": {
      "command": "/usr/local/bin/zeroclaw-coordinator-mcp",
      "args": ["--transport", "stdio"]
    }
  }
}
```

> [!IMPORTANT]
> For `stdio` to work between two Docker containers, the client container would need access to the host's Docker socket, which is not recommended for security reasons. Stick to **SSE** for a seamless experience.

This server is designed to work as the primary management backend for the [ZeroClaw Home Assistant Add-on](https://github.com/paul-crafts/zeroclaw-ha-addon).

---

## 🛠️ Features

- **Recursive Workspace Management:** List and navigate deep directory structures with ease.
- **Safe File Operations:** Automatically handles directory creation and validation.
- **Secure by Design:** Configurable whitelist and substring-based blacklist to protect sensitive files.
- **Real-time Coordination:** Supports both `stdio` and `SSE` transports for flexible integration.
- **Optimized for Home Assistant:** Minimal footprint, built for speed and reliability.

---

## ⚙️ Configuration

### Environment Variables (Standalone)
When running the standalone binary, you can manage the configuration via environment variables:

- **`ZEROCLAW_WORKSPACE`**: Path to the workspace directory.
- **`ZEROCLAW_WHITELIST`**: Comma-separated list of allowed directories.
- **`ZEROCLAW_BLACKLIST`**: Comma-separated list of substrings to block.

Example for full Home Assistant management:
```bash
export ZEROCLAW_WORKSPACE="/config"
export ZEROCLAW_WHITELIST="/config,/share"
export ZEROCLAW_BLACKLIST="secrets.yaml,IDENTITY.md,id_rsa,.env"
./zeroclaw-coordinator-mcp
```

> [!CAUTION]
> When setting the workspace to `/config`, ensure your **Blacklist** includes all sensitive credential files (like `secrets.yaml`) to prevent the LLM from accessing your private keys or passwords.

Check the [DOCS.md](DOCS.md) for more details.

---

## 💎 Support

If you find this project useful, consider supporting the development!

[![Buy Me A Coffee](https://img.shields.io/badge/Buy%20Me%20a%20Coffee-ffdd00?style=for-the-badge&logo=buy-me-a-coffee&logoColor=black)](https://www.buymeacoffee.com/paul.crafts)

---

## 📜 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
