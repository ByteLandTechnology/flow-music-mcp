# flow-music-mcp

[![English](https://img.shields.io/badge/English-64748b?style=flat-square&logoWidth=16&logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAxOCI%2BPGNsaXBQYXRoIGlkPSJ1Ij48cmVjdCB3aWR0aD0iMjQiIGhlaWdodD0iMTgiIHJ4PSIyIi8%2BPC9jbGlwUGF0aD48ZyBjbGlwLXBhdGg9InVybCgjdSkiPjxyZWN0IHdpZHRoPSIyNCIgaGVpZ2h0PSIxOCIgZmlsbD0iI2ZmZiIvPjxwYXRoIHN0cm9rZT0iI2IyMjIzNCIgc3Ryb2tlLXdpZHRoPSIxLjQiIGQ9Ik0wIC43aDI0TTAgMy41aDI0TTAgNi4zaDI0TTAgOS4xaDI0TTAgMTEuOWgyNE0wIDE0LjdoMjRNMCAxNy4zaDI0Ii8%2BPHJlY3Qgd2lkdGg9IjEwLjQiIGhlaWdodD0iOS44IiBmaWxsPSIjM2MzYjZlIi8%2BPHBhdGggc3Ryb2tlPSIjZmZmIiBzdHJva2Utd2lkdGg9IjEuMyIgc3Ryb2tlLWxpbmVjYXA9InJvdW5kIiBkPSJNMi4xIDIuMWguMDFNNS4yIDIuMWguMDFNOC4zIDIuMWguMDFNMy42IDQuOWguMDFNNi44IDQuOWguMDFNMi4xIDcuN2guMDFNNS4yIDcuN2guMDFNOC4zIDcuN2guMDEiLz48L2c%2BPC9zdmc%2B)](../../README.md)
![简体中文](https://img.shields.io/badge/%E7%AE%80%E4%BD%93%E4%B8%AD%E6%96%87-2563eb?style=flat-square&logoWidth=16&logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAxOCI%2BPHJlY3Qgd2lkdGg9IjI0IiBoZWlnaHQ9IjE4IiByeD0iMiIgZmlsbD0iI2RlMjkxMCIvPjxwYXRoIGZpbGw9IiNmZmRlMDAiIGQ9Ik00LjggMi40IDYuMyA3LjEgMi4zIDQuMiA3LjMgNC4yIDMuMyA3LjF6TTguNCAyLjYgOS41IDEuMyA5LjQgMy4wIDguNSAxLjUgMTAuMSAyLjF6TTEwLjAgNC4wIDExLjUgMy4xIDEwLjggNC43IDEwLjUgMy4wIDExLjcgNC4yek0xMC4wIDYuMiAxMS43IDYuMCAxMC40IDcuMiAxMC44IDUuNSAxMS41IDcuMXpNOC41IDcuNiAxMC4xIDguMSA4LjUgOC42IDkuNSA3LjIgOS41IDkuMHoiLz48L3N2Zz4%3D)
[![日本語](https://img.shields.io/badge/%E6%97%A5%E6%9C%AC%E8%AA%9E-64748b?style=flat-square&logoWidth=16&logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAxOCI%2BPHJlY3Qgd2lkdGg9IjI0IiBoZWlnaHQ9IjE4IiByeD0iMiIgZmlsbD0iI2ZmZiIvPjxjaXJjbGUgY3g9IjEyIiBjeT0iOSIgcj0iNSIgZmlsbD0iI2JjMDAyZCIvPjwvc3ZnPg%3D%3D)](../ja/README.md)

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
      "args": ["-y", "@flow-music-mcp/cli", "serve", "--home", "/path/to/.flow-music-mcp"]
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
> 项目状态：npm 包和 GitHub Releases 尚未发布，目前请从源代码构建。
