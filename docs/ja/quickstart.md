# クイックスタート

このガイドに従って、`flow-music-mcp` をインストールし、MCP クライアントを接続し、Google Flow Music にログインして、曲を作成します。

## 前提条件

- **Google Chrome または Chromium**: サーバーの実行時に Chrome または Chromium が必要です。自動検出でブラウザーが見つからない場合は、`--chrome <PATH>` または `FLOW_MUSIC_MCP_CHROME` を設定してください。
- **Google Flow Music にアクセスできる Google アカウント**: そのアカウントで [Google Flow Music](https://www.flowmusic.app) を開けること、およびクレジットがあることを確認してください。
- **Node.js `18` 以降**: Node.js は `@flow-music-mcp/cli` を実行します。バイナリをソースからビルドすることもできます。

## 1. インストール

```sh
npx -y @flow-music-mcp/cli --help
```

または、ソースからビルドします。

```sh
git clone https://github.com/ByteLandTechnology/flow-music-mcp.git
cd flow-music-mcp
npm ci
cargo build --release
./target/release/flow-music-mcp --help
```

## 2. MCP クライアントの設定

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

`--home` ディレクトリには、Chrome プロファイル、SQLite インデックス、すべてのダウンロードが保存されます。既定値は `~/.flow-music-mcp` です。

## 3. ログイン

`{"action": "login"}` を指定して `music_login` を呼び出します。可視の Chrome ウィンドウで Google Flow Music が開きます。

1. **Log in** を選択し、Google ログインを完了します。
2. サイトに年齢確認とウェルカム画面が表示された場合は、それらを完了します。
3. ウィンドウを閉じるか、そのまま待ちます。サーバーはセッションを検出するとウィンドウを閉じ、同じプロファイルでヘッドレス Chrome を起動します。

次に、`{"probe": true}` を指定して `music_status` を呼び出します。結果には `signed_in: true`、プラン、クレジットが表示されます。

ターミナルから `flow-music-mcp login --home /path/to/.flow-music-mcp` を実行してログインすることもできます。2 つの `flow-music-mcp` プロセスは同じ Chrome プロファイルを同時に使用できないため、先に MCP クライアントを停止してください。`flow-music-mcp doctor` を実行すると、Chrome バイナリ、データディレクトリ、ログイン状態を確認できます。

## 4. 曲の作成

`music_generate` を呼び出します。

```json
{
  "prompt": "warm lo-fi hip hop with soft piano and vinyl crackle",
  "instrumental": true,
  "title": "Night Study"
}
```

ツールは 8 文字の `id` を持つジョブを返します。その `id` を指定して `music_wait` を呼び出します。1 回の呼び出しで最大 45 秒待機します。結果に `pending: true` が含まれる場合は、もう一度 `music_wait` を呼び出してください。

ジョブが `done` になると、結果には次の情報が含まれます。

- `created`: 新しいクリップ ID。
- `files`: `<home>/downloads` 内のローカルオーディオファイル。
- `resources`: クライアントが読み取れる `music://downloads/<id>` URI。
- `result.clips`: タイトル、長さ、歌詞、公開オーディオ URL。

## 5. 次のステップ

- `music_lyrics` で歌詞を作成し、`lyrics_id` を `music_generate` に渡します。
- `music_edit` と `{"action": "extend", "clip_id": "...", "extend_s": 30}` で曲を延長します。
- `music_stems` でステムを分離します。
- `music_download` と `{"id": "...", "format": "wav"}` で WAV または MP3 をダウンロードします。
- その他の作業は、`music_chat` で Producer エージェントに依頼します。

すべてのパラメーターについては、[ツールリファレンス](tools-reference.md)を参照してください。
