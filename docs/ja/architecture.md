# アーキテクチャ

このガイドでは、`flow-music-mcp` の仕組みを説明します。

## コンポーネント

| コンポーネント | ソース | 役割 |
| :--- | :--- | :--- |
| MCP アダプター | `src/mcp.rs` | `rmcp` で 18 個のツールと `music://downloads/...` リソースを登録します。時間のかかるツールは 45 秒後に `pending: true` を返します。 |
| サービス | `src/service.rs` | ツールを振り分け、ジョブをキューに入れ、ダウンロードとリソースを提供します。 |
| `ChromeEngine` | `src/engine.rs`、`src/chrome.rs`、`src/cdp.rs` | Chrome を起動し、アプリケーションのオリジン上にページを 1 つ保持し、ページランタイムを実行します。 |
| ページランタイム | `ts/*.ts` | ページのセッションを読み取り、API リクエスト、ストリーム読み取り、アップロード、バイナリダウンロードを送信します。 |
| API クライアント | `src/client.rs` | ページランタイムを呼び出し、期限切れのセッションをページの再読み込みで 1 回更新します。 |
| Producer レイヤー | `src/producer.rs` | Producer ツール呼び出しとチャットメッセージを送信し、ジョブストリームを追跡し、クリップのレンダリング完了を待ちます。 |
| ドメインモジュール | `src/generation.rs`、`src/edits.rs`、`src/chat.rs`、`src/library.rs`、`src/collections.rs`、`src/uploads.rs` | 各ツールのリクエストを構築します。 |
| `JobQueue` | `src/queue.rs` | クレジットを消費するジョブを 1 つずつ実行します。 |
| ストレージ | `src/db.rs`、`src/download.rs`、`src/media_files.rs` | ファイルを `<home>/downloads` に保存し、SQLite でインデックスを作成します。 |

## ログインとセッション

Google Flow Music は、`www.flowmusic.app` 上の Supabase Cookie（`sb-sb-auth-token.0`、`sb-sb-auth-token.1`、およびその他のチャンク）にセッションを保持します。セッションは `<home>/chrome-profile` の永続的な Chrome プロファイル内にのみ存在します。サーバーはセッション Cookie を他のファイルにコピーしません。

1. `music_login` は、`<home>/chrome-profile` で CDP フラグなしの可視の Chrome ウィンドウを開きます。
2. サーバーは毎秒、Chrome の `Cookies` データベースと `History` データベースの一時コピーを読み取ります。セッション Cookie と、Google アカウントの拒否ページを探します。
3. セッション Cookie が存在するとき、またはユーザーがウィンドウを閉じたとき、サーバーは可視の Chrome を停止し、同じプロファイルでヘッドレス Chrome を起動します。
4. ページランタイムは `GET /__api/users/me` でセッションを確認します。

アクセストークンは約 1 時間で期限切れになります。ランタイムが期限切れのトークンを検出したとき、または API が HTTP 401 を返したとき、クライアントはページを再読み込みします。その後、ページのクライアントがトークンを更新し、クライアントはリクエストを 1 回だけ再送信します。

### 単一インスタンスロックと孤立 Chrome

- Chrome を起動する前に、`flow-music-mcp` は `<home>/flow-music-mcp.lock` に排他ロックを取得し、プロセス終了まで保持します。プロセスが終了すると（クラッシュ時も含む）、OS がロックを解放します。
- 別のプロセスがロックを保持している場合、Chrome の起動、`login`、`status`、`logout`、および `music_login` の `logout` はエラーになります。`doctor` は警告を表示します。
- ロックを保持している間、`<home>/chrome-profile` 上でこのプロセスが起動していない Chrome プロセスは孤立プロセスです。`flow-music-mcp` はまず通常終了を要求し（Unix では `SIGINT`、Windows では `taskkill`）、3 秒後も残っている場合は強制終了します。

## ページからのリクエスト

内部 API は `https://www.flowmusic.app/__api/...` にあります。各リクエストには `Authorization: Bearer <access token>` が付きます。ランタイムはすべてのリクエストをページから `fetch` で送信するため、リクエストはウェブサイトと同じオリジン、Cookie、ブラウザー識別情報を持ちます。

Rust 側は固定の API パスのみを使用します。呼び出し元が選んだパスにリクエストを送信できるツールはありません。

## Producer ツール呼び出し

曲の作成、編集、ステム、歌詞、動画は Producer ツールです。

1. `POST /__api/producer/tool-call` は 1 つのツール呼び出しを送信し、`job_id` を返します。
2. `GET /__api/messages/<job_id>/stream` は Server-Sent Events を送信します。ランタイムはストリームを 20 秒ごとに区切って読み取り、`last_id` で続きを読み取ります。
3. 同じ `tool_call_id` を持つ `tool-return` パートが、新しいクリップ ID などの結果を運びます。
4. ツールが返った後も、クリップのレンダリングが続く場合があります。サーバーは各クリップのオーディオが完成するまで `POST /__api/clips` をポーリングします。

編集にはレシピを使用します。`POST /__api/recipes/create` は 3 つのノードを保存します。入力クリップ、`ExtendSection`、`ReplaceSection`、または `CoverSong` ノード、そして出力です。その後、`audio__render_edit` ツールがレシピをレンダリングします。

`music_generate` は、ウェブサイトの作曲フォームと同じ引数を構築します。BPM はサウンドプロンプトに入り、長さは `[End - m:ss]` マーカーとして歌詞に入ります。歌詞 ID は歌詞テキストの UUID バージョン 5 で、歌詞テキストはクライアントコンテキストに入ります。モデルは歌詞、BPM、長さを目安として使用します。

`music_chat` は `POST /__api/conversation` を送信します。Producer エージェントは自分でツールを選択するため、チャットメッセージがクレジットを消費する場合があります。

## ジョブキュー

`JobQueue` は 1 つのワーカータスクでジョブを 1 つずつ実行します。ジョブとジョブの間に、ワーカーは `--pause` または `FLOW_MUSIC_MCP_PAUSE` で指定した範囲のランダムな時間（既定値は 5〜15 秒）待機します。

- キューはジョブを自動で再試行しません。`music_retry` は失敗またはキャンセルされたジョブを再実行し、再度クレジットを消費します。
- `music_cancel` はローカルのジョブを停止します。ジョブにセッションがある場合、サーバーは `POST /__api/conversations/<id>/stop` も送信します。ツール呼び出しがすでに開始した clip はサーバー上でレンダリングを続け、credits を消費します。
- キューは直近 200 件のジョブをメモリに保持します。サーバーを再起動するとキューは消去されますが、曲はライブラリに残ります。

## ストレージ

すべてのデータは `<home>`（既定値は `~/.flow-music-mcp`）の下にあります。

| パス | 内容 | Unix パーミッション |
| :--- | :--- | :--- |
| `chrome-profile/` | セッションを含む永続的な Chrome プロファイル | `0700` |
| `flow-music-mcp.lock` | Chrome を起動できる間にプロセスが保持する単一インスタンスロック | `0600` |
| `flow-music-mcp.db` | ダウンロードの SQLite インデックス | `0600` |
| `downloads/` | ダウンロードしたオーディオ、動画、画像 | `0700` ディレクトリ |

M4A、WAV、ステム、カバーのファイルは、公開ストレージの URL から取得します。MP3 ファイルは、ページを介して `GET /__api/download/audio/<clip_id>?format=mp3` から取得します。サーバーは、各ファイルのインデックスを作成する前に、ファイルの先頭バイトを確認します。
