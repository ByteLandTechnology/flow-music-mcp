# flow-music-mcp

[![English](https://img.shields.io/badge/English-64748b?style=flat-square&logoWidth=16&logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAxOCI%2BPGNsaXBQYXRoIGlkPSJ1Ij48cmVjdCB3aWR0aD0iMjQiIGhlaWdodD0iMTgiIHJ4PSIyIi8%2BPC9jbGlwUGF0aD48ZyBjbGlwLXBhdGg9InVybCgjdSkiPjxyZWN0IHdpZHRoPSIyNCIgaGVpZ2h0PSIxOCIgZmlsbD0iI2ZmZiIvPjxwYXRoIHN0cm9rZT0iI2IyMjIzNCIgc3Ryb2tlLXdpZHRoPSIxLjQiIGQ9Ik0wIC43aDI0TTAgMy41aDI0TTAgNi4zaDI0TTAgOS4xaDI0TTAgMTEuOWgyNE0wIDE0LjdoMjRNMCAxNy4zaDI0Ii8%2BPHJlY3Qgd2lkdGg9IjEwLjQiIGhlaWdodD0iOS44IiBmaWxsPSIjM2MzYjZlIi8%2BPHBhdGggc3Ryb2tlPSIjZmZmIiBzdHJva2Utd2lkdGg9IjEuMyIgc3Ryb2tlLWxpbmVjYXA9InJvdW5kIiBkPSJNMi4xIDIuMWguMDFNNS4yIDIuMWguMDFNOC4zIDIuMWguMDFNMy42IDQuOWguMDFNNi44IDQuOWguMDFNMi4xIDcuN2guMDFNNS4yIDcuN2guMDFNOC4zIDcuN2guMDEiLz48L2c%2BPC9zdmc%2B)](../../README.md)
[![简体中文](https://img.shields.io/badge/%E7%AE%80%E4%BD%93%E4%B8%AD%E6%96%87-64748b?style=flat-square&logoWidth=16&logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAxOCI%2BPHJlY3Qgd2lkdGg9IjI0IiBoZWlnaHQ9IjE4IiByeD0iMiIgZmlsbD0iI2RlMjkxMCIvPjxwYXRoIGZpbGw9IiNmZmRlMDAiIGQ9Ik00LjggMi40IDYuMyA3LjEgMi4zIDQuMiA3LjMgNC4yIDMuMyA3LjF6TTguNCAyLjYgOS41IDEuMyA5LjQgMy4wIDguNSAxLjUgMTAuMSAyLjF6TTEwLjAgNC4wIDExLjUgMy4xIDEwLjggNC43IDEwLjUgMy4wIDExLjcgNC4yek0xMC4wIDYuMiAxMS43IDYuMCAxMC40IDcuMiAxMC44IDUuNSAxMS41IDcuMXpNOC41IDcuNiAxMC4xIDguMSA4LjUgOC42IDkuNSA3LjIgOS41IDkuMHoiLz48L3N2Zz4%3D)](../zh-CN/README.md)
![日本語](https://img.shields.io/badge/%E6%97%A5%E6%9C%AC%E8%AA%9E-2563eb?style=flat-square&logoWidth=16&logo=data%3Aimage%2Fsvg%2Bxml%3Bbase64%2CPHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAxOCI%2BPHJlY3Qgd2lkdGg9IjI0IiBoZWlnaHQ9IjE4IiByeD0iMiIgZmlsbD0iI2ZmZiIvPjxjaXJjbGUgY3g9IjEyIiBjeT0iOSIgcj0iNSIgZmlsbD0iI2JjMDAyZCIvPjwvc3ZnPg%3D%3D)

`flow-music-mcp` は、Google Labs (Lyria) の AI 音楽スタジオ [Google Flow Music](https://www.flowmusic.app) 用の、単一バイナリの Rust 製 stdio Model Context Protocol (MCP) サーバーです。`rmcp` を使用して、標準入出力で 18 個の MCP ツールとダウンロードリソースを提供します。

`ChromeEngine` は、永続的な Chrome または Chromium のプロファイルを `<home>/chrome-profile` に保持します（Unix ではパーミッション `0700`）。バックグラウンド処理中は、ローカルの Chrome DevTools Protocol (CDP) でヘッドレス Chrome（`--headless=new`）に接続し、`https://www.flowmusic.app` 上にページを 1 つ保持します。組み込みの TypeScript ランタイムが、そのページのセッションを使用して、そのページから内部 API リクエストを送信します。`music_login` での Google ログイン中は、サーバーは CDP フラグなしの可視の Chrome ウィンドウを開きます。

シングルワーカーの `JobQueue` は、生成、歌詞、編集、ステム、動画、チャットのジョブを 1 つずつ実行します。ジョブはクレジットを消費するため、キューはジョブを自動で再試行しません。完成したオーディオは自然なファイル名で `<home>/downloads` に保存され、SQLite インデックスが各ファイルを記録します。

## クイックインストール

ホストコンピューターに Node.js `18` 以降と、Google Chrome または Chromium をインストールします。

```sh
npx -y @flow-music-mcp/cli --help
```

`npm` は一致するオプションパッケージ（`@flow-music-mcp/darwin-arm64`、`@flow-music-mcp/darwin-x64`、`@flow-music-mcp/linux-arm64`、`@flow-music-mcp/linux-x64`、`@flow-music-mcp/win32-arm64`、または `@flow-music-mcp/win32-x64`）をインストールします。`--omit=optional` や `--no-optional` は指定しないでください。

ソースからビルドするには、Rust `1.88`（以降）と Node.js をインストールします。`esbuild` がコンパイル時に `ts/index.ts` をバンドルするため、バイナリの実行時に Node.js は不要です。

```sh
git clone https://github.com/ByteLandTechnology/flow-music-mcp.git
cd flow-music-mcp
npm ci
cargo build --release
```

MCP stdio クライアントの設定（`mcpServers`）にサーバーを追加します。

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

## ドキュメント一覧

| ガイド | 種類 | 主な内容 |
| :--- | :--- | :--- |
| [クイックスタート](quickstart.md) | チュートリアル | インストール、クライアントの設定、ログイン、最初の曲の作成 |
| [アーキテクチャ](architecture.md) | 解説 | `ChromeEngine`、ページランタイム、Producer ツール呼び出し、`JobQueue`、ストレージ |
| [ツールリファレンス](tools-reference.md) | リファレンス | コマンドラインオプション、環境変数、MCP リソース、すべての MCP ツール |

## ツールの概要

| 領域 | ツール | 目的 |
| :--- | :--- | :--- |
| セッション | `music_status`、`music_login` | セッション、クレジット、プランを確認し、Chrome でログインします |
| 作成（キュー） | `music_generate`、`music_lyrics`、`music_edit`、`music_stems`、`music_chat`、`music_video` | 曲と歌詞を作成し、曲の延長、置換、カバーを行い、ステムを分離し、Producer エージェントと対話し、動画を作成します |
| キュー制御 | `music_wait`、`music_cancel`、`music_retry` | ジョブを待機し（1 回の呼び出しで 45 秒）、ジョブを停止し、失敗したジョブを再実行します |
| ライブラリ | `music_songs`、`music_sessions`、`music_playlists`、`music_spaces`、`music_upload` | 曲、セッション、プレイリスト、Spaces を管理し、オーディオや画像をアップロードします |
| ファイル | `music_download`、`music_downloads` | オーディオ（m4a、wav、mp3）、動画、カバーアートをダウンロードし、ローカルファイルを管理します |

> [!IMPORTANT]
> `flow-music-mcp` は Google Flow Music の内部インターフェースを使用します。これらのインターフェースは予告なく変更される場合があります。生成ツールは Google Flow Music プランのクレジットを消費します。

> [!NOTE]
> プロジェクトの状態：npm パッケージと GitHub Releases はまだ公開されていないため、現在はソースからビルドしてください。
