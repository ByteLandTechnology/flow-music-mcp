# 快速入门

按照本指南安装 `flow-music-mcp`、连接 MCP 客户端、登录 Google Flow Music 并创作一首歌曲。

## 前提条件

- **Google Chrome 或 Chromium**：服务器在运行时需要 Chrome 或 Chromium。如果自动检测找不到您的浏览器，请设置 `--chrome <PATH>` 或 `FLOW_MUSIC_MCP_CHROME`。
- **可以访问 Google Flow Music 的 Google 账号**：请确保该账号可以打开 [Google Flow Music](https://www.flowmusic.app) 并且有可用额度。
- **Node.js `18` 或更高版本**：Node.js 用于运行 `@flow-music-mcp/cli`。您也可以从源码构建二进制文件。

## 1. 安装

```sh
npx -y @flow-music-mcp/cli --help
```

或者从源码构建：

```sh
git clone https://github.com/ByteLandTechnology/flow-music-mcp.git
cd flow-music-mcp
npm ci
cargo build --release
./target/release/flow-music-mcp --help
```

## 2. 配置 MCP 客户端

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

`--home` 目录存放 Chrome 配置文件、SQLite 索引和所有下载文件。默认值为 `~/.flow-music-mcp`。

## 3. 登录

使用 `{"action": "login"}` 调用 `music_login`。一个可见的 Chrome 窗口会打开 Google Flow Music：

1. 选择 **Log in** 并完成 Google 登录。
2. 如果网站显示年龄验证和欢迎步骤，请完成这些步骤。
3. 关闭窗口，或等待。服务器检测到会话后会关闭窗口，并在同一配置文件上启动无头 Chrome。

然后使用 `{"probe": true}` 调用 `music_status`。结果会显示 `signed_in: true`、您的套餐和额度。

也可以在终端中运行 `flow-music-mcp login --home /path/to/.flow-music-mcp` 完成登录。请先停止 MCP 客户端，因为两个 `flow-music-mcp` 进程不能同时使用同一个 Chrome 配置文件。运行 `flow-music-mcp doctor` 可以检查 Chrome 二进制文件、数据目录和登录状态。

## 4. 创作歌曲

调用 `music_generate`：

```json
{
  "prompt": "warm lo-fi hip hop with soft piano and vinyl crackle",
  "instrumental": true,
  "title": "Night Study"
}
```

该工具返回一个带有八个字符 `id` 的作业。使用该 `id` 调用 `music_wait`。每次调用最多等待 45 秒。如果结果中有 `pending: true`，请再次调用 `music_wait`。

当作业状态为 `done` 时，结果包含：

- `created`：新的片段 ID。
- `files`：`<home>/downloads` 中的本地音频文件。
- `resources`：客户端可以读取的 `music://downloads/<id>` URI。
- `result.clips`：标题、时长、歌词和公开的音频 URL。

## 5. 后续操作

- 使用 `music_lyrics` 创作歌词，然后将 `lyrics_id` 传给 `music_generate`。
- 使用 `music_edit` 和 `{"action": "extend", "clip_id": "...", "extend_s": 30}` 延长歌曲。
- 使用 `music_stems` 分离音轨。
- 使用 `music_download` 和 `{"id": "...", "format": "wav"}` 下载 WAV 或 MP3。
- 其他任何需求，请使用 `music_chat` 询问 Producer 代理。

所有参数请参阅[工具参考](tools-reference.md)。
