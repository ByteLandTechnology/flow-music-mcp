# flow-music-mcp

[English](../../README.md) | [简体中文](../zh-CN/README.md) | [日本語](../ja/README.md)

`flow-music-mcp` 是一个单二进制的 Rust stdio Model Context Protocol (MCP) 服务器，面向 Google Labs (Lyria) 推出的 AI 音乐工作室 [Google Flow Music](https://www.flowmusic.app)。它通过 `rmcp` 在标准输入和标准输出上提供 18 个 MCP 工具和下载资源。

`ChromeEngine` 在 `<home>/chrome-profile` 中保存持久化的 Chrome 或 Chromium 配置文件（在 Unix 上权限为 `0700`）。在后台工作期间，它通过本地 Chrome DevTools Protocol (CDP) 连接到无头 Chrome（`--headless=new`），并在 `https://www.flowmusic.app` 上保持一个页面。内嵌的 TypeScript 运行时使用该页面的会话，从该页面发送内部 API 请求。在 `music_login` 中进行 Google 登录时，服务器会打开一个不带 CDP 参数的可见 Chrome 窗口。

单工作线程的 `JobQueue` 逐个运行生成、歌词、编辑、分轨、视频和对话作业。由于作业会消耗额度，队列从不自动重试作业。完成的音频以自然文件名保存到 `<home>/downloads`，并由 SQLite 索引记录每个文件。

## 快速安装

在主机上安装 Node.js `18` 或更高版本，以及 Google Chrome 或 Chromium：

```sh
npx -y @flow-music-mcp/cli --help
```

`npm` 会安装匹配的可选包（`@flow-music-mcp/darwin-arm64`、`@flow-music-mcp/darwin-x64`、`@flow-music-mcp/linux-arm64`、`@flow-music-mcp/linux-x64`、`@flow-music-mcp/win32-arm64` 或 `@flow-music-mcp/win32-x64`）。请勿传入 `--omit=optional` 或 `--no-optional`。

如需从源码构建，请安装 Rust `1.88`（或更高版本）和 Node.js。`esbuild` 在编译时打包 `ts/index.ts`，因此二进制文件在运行时不需要 Node.js：

```sh
git clone https://github.com/ByteLandTechnology/flow-music-mcp.git
cd flow-music-mcp
npm ci
cargo build --release
```

将服务器添加到 MCP stdio 客户端配置（`mcpServers`）中：

```json
{
  "mcpServers": {
    "flow-music": {
      "command": "npx",
      "args": ["-y", "@flow-music-mcp/cli", "--home", "/path/to/.flow-music-mcp"]
    }
  }
}
```

## 文档导航

| 指南 | 类型 | 主要内容 |
| :--- | :--- | :--- |
| [快速入门](quickstart.md) | 教程 | 安装、配置客户端、登录并创作第一首歌曲 |
| [架构](architecture.md) | 说明 | `ChromeEngine`、页面运行时、Producer 工具调用、`JobQueue` 和存储 |
| [工具参考](tools-reference.md) | 参考 | 命令行选项、环境变量、MCP 资源和所有 MCP 工具 |

## 工具概览

| 领域 | 工具 | 用途 |
| :--- | :--- | :--- |
| 会话 | `music_status`、`music_login` | 检查会话、额度和套餐，并通过 Chrome 登录 |
| 创作（排队） | `music_generate`、`music_lyrics`、`music_edit`、`music_stems`、`music_chat`、`music_video` | 创作歌曲和歌词，延长、替换或翻唱歌曲，分离音轨，与 Producer 代理对话，以及制作视频 |
| 队列控制 | `music_wait`、`music_cancel`、`music_retry` | 等待作业（每次调用 45 秒）、停止作业或重新运行失败的作业 |
| 资料库 | `music_songs`、`music_sessions`、`music_playlists`、`music_spaces`、`music_upload` | 管理歌曲、会话、播放列表和 Spaces，并上传音频或图片 |
| 文件 | `music_download`、`music_downloads` | 下载音频（m4a、wav、mp3）、视频或封面图片，并管理本地文件 |

> [!IMPORTANT]
> `flow-music-mcp` 使用 Google Flow Music 的内部接口。这些接口可能会在不另行通知的情况下发生变化。生成类工具会消耗您 Google Flow Music 套餐中的额度。

> [!NOTE]
> 项目状态：npm 包和 GitHub Releases 尚未发布，目前请从源代码构建。`music_video` 的视频创建与视频下载，以及 `music_cancel` 对运行中作业的服务器端停止，尚未在线上服务中验证。
