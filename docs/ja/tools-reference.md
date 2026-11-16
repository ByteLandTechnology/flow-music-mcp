# ツールリファレンス

このリファレンスでは、`flow-music-mcp` のコマンドラインオプション、環境変数、MCP リソース、MCP ツールを示します。

## コマンドラインオプション

| オプション | 説明 |
| :--- | :--- |
| `--home <PATH>` | データのルートディレクトリ。既定値: `FLOW_MUSIC_MCP_HOME`、次に `~/.flow-music-mcp`。 |
| `--chrome <PATH>` | Chrome または Chromium のバイナリ。既定値: `FLOW_MUSIC_MCP_CHROME`、次に自動検出。 |

## 環境変数

| 変数 | 説明 |
| :--- | :--- |
| `FLOW_MUSIC_MCP_HOME` | `--home` が設定されていない場合のデータのルートディレクトリ。 |
| `FLOW_MUSIC_MCP_CHROME` | `--chrome` が設定されていない場合の Chrome または Chromium のバイナリ。 |
| `FLOW_MUSIC_MCP_CHROME_CDP_PORT` | 固定のローカル CDP ポート。既定値: Chrome が選ぶ空きポート。 |
| `FLOW_MUSIC_MCP_PAUSE_MIN_S` | キュー内のジョブ間の最小待機時間（秒）。既定値: `5`。 |
| `FLOW_MUSIC_MCP_PAUSE_MAX_S` | キュー内のジョブ間の最大待機時間（秒）。既定値: `15`。 |

## MCP リソース

| URI | 内容 |
| :--- | :--- |
| `music://downloads/{id}` | SQLite レコード ID で指定するダウンロード済みファイル。 |
| `music://downloads/asset/{asset_id}` | クリップ ID または動画 ID の最新のダウンロード済みファイル。 |

`resources/read` は 64 MiB までのファイルを返します。それより大きいファイルには、`music_downloads` のアクション `read_chunk` を使用してください。

## ジョブ

キュー型のツールは、8 文字の `id` を持つジョブオブジェクトを返します。結果を取得するには `music_wait` を呼び出します。終了したジョブの `status` は `done`、`failed`、または `cancelled` で、次のフィールドを持ちます。

| フィールド | 説明 |
| :--- | :--- |
| `created` | 新しいクリップ、歌詞、または動画の ID。 |
| `files` | ダウンロードのローカルファイルパス。 |
| `resources` | ダウンロードの `music://downloads/{id}` URI。 |
| `result` | ジョブの種類に固有のデータ: クリップの概要、歌詞、Producer の返信、または動画データ。 |
| `conversation_id` | ジョブが使用したセッション。 |
| `error` | 失敗したジョブのエラーテキスト。 |

## セッション

### `music_status`

Google Flow Music のセッション状態、ジョブキュー、ダウンロードインデックス、ブラウザーの状態を報告します。probe=true を設定すると、Chrome を起動してアカウント、プラン、クレジットを読み取ります。

| パラメーター | 型 | 必須 | 説明 |
| :--- | :--- | :--- | :--- |
| `probe` | boolean | いいえ | 必要に応じて Chrome を起動し、アカウント、プラン、クレジットを読み取ります（既定値 false）。 |

### `music_login`

可視の Google Chrome ウィンドウで Google Flow Music にログインする（アクション "login"）、セッションを報告する（"status"）、または保存済みのセッションを削除します（"logout"）。

| パラメーター | 型 | 必須 | 説明 |
| :--- | :--- | :--- | :--- |
| `action` | string | いいえ | アクション名: "login"（別名 "start"）、"status"、または "logout"（別名 "reset_session"）。 |
| `force` | boolean | いいえ | 対話型ログインの開始前に、保存済みの Chrome プロファイルを削除します（既定値 false）。 |
| `timeout_secs` | integer | いいえ | ログイン完了までの最大待機時間（秒）（既定値 180、範囲 15〜3600）。 |
| `wait` | boolean | いいえ | "login" の実行中、Chrome でログインが完了するまで待機します（既定値 true）。 |

## 作成ジョブ

### `music_generate`

Lyria による曲の生成ジョブをキューに入れます（クレジットを消費します）。サウンドプロンプトと、任意で歌詞、インストゥルメンタル、BPM、長さ、タイトル、シード、プロンプト画像を指定します。歌詞、BPM、長さはヒントです。ジョブは終了時にオーディオをダウンロードします。結果を取得するには music_wait を使用します。

| パラメーター | 型 | 必須 | 説明 |
| :--- | :--- | :--- | :--- |
| `bpm` | integer | いいえ | 1 分あたりの拍数で示すテンポのヒント。この値はサウンドプロンプトに入ります。 |
| `download` | boolean | いいえ | 完成したオーディオをダウンロードします（既定値 true）。 |
| `format` | string | いいえ | ダウンロード形式: "m4a"（既定値）、"wav"、または "mp3"。 |
| `image_id` | string | いいえ | プロンプト画像として使用する、`music_upload` から取得した画像 ID。 |
| `instrumental` | boolean | いいえ | ボーカルのないインストゥルメンタル曲を作成します（既定値 false）。 |
| `length_s` | integer | いいえ | 秒単位の長さのヒント。モデルが異なる長さで作成する場合があります。 |
| `lyrics` | string | いいえ | 歌詞テキスト。モデルは歌詞を目安として使用し、変更する場合があります。省略するとモデルが歌詞を書きます。 |
| `lyrics_id` | string | いいえ | `music_lyrics` から取得した歌詞 ID。設定すると、ツールは `lyrics` と `length_s` を無視します。 |
| `prompt` | string | はい | サウンドの説明: ジャンル、楽器、ムード、ボーカルスタイル。 |
| `seed` | integer | いいえ | 再現可能な結果を得るためのシード。 |
| `session_id` | string | いいえ | 既存のセッション ID。省略すると新しいセッションを作成します。 |
| `title` | string | いいえ | 曲のタイトル（既定値 "Untitled"）。 |

### `music_lyrics`

歌詞ジョブをキューに入れます。プロンプトから新しい歌詞を書くか、既存の歌詞を編集します。結果には、歌詞テキスト、タイトル、それに合うサウンドプロンプトが含まれます。

| パラメーター | 型 | 必須 | 説明 |
| :--- | :--- | :--- | :--- |
| `action` | string | いいえ | アクション: "write"（既定値）は `prompt` から新しい歌詞を作成します。"edit" は `prompt` に従って `lyrics` または `lyrics_id` を変更します。 |
| `lyrics` | string | いいえ | "edit" で編集する既存の歌詞テキスト。 |
| `lyrics_id` | string | いいえ | "edit" で編集する既存の歌詞 ID。 |
| `prompt` | string | はい | 作詞者への指示。 |
| `session_id` | string | いいえ | 既存のセッション ID。省略すると新しいセッションを作成します。 |

### `music_edit`

曲の編集をキューに入れます（クレジットを消費します）。"extend" は新しいオーディオを追加し、"replace" は時間範囲を書き直し、"cover" はメロディーを保ったまま曲のスタイルを変えます。結果は新しいクリップです。

| パラメーター | 型 | 必須 | 説明 |
| :--- | :--- | :--- | :--- |
| `action` | string | はい | 編集アクション: "extend"、"replace"、または "cover"。 |
| `clip_id` | string | はい | ソースクリップ ID。 |
| `download` | boolean | いいえ | 完成したオーディオをダウンロードします（既定値 true）。 |
| `extend_from_s` | number | いいえ | "extend" の開始位置（秒）。既定値: クリップの末尾。 |
| `extend_s` | number | いいえ | "extend" で追加する新しいオーディオの秒数（既定値 30）。 |
| `format` | string | いいえ | ダウンロード形式: "m4a"（既定値）、"wav"、または "mp3"。 |
| `instruction` | string | いいえ | 編集の指示。例: "add a guitar solo"。既定値: アクションの汎用的な指示。 |
| `regions` | array of Region | いいえ | "replace" で書き直す範囲。 |
| `seed` | integer | いいえ | 再現可能な結果を得るためのシード。 |
| `session_id` | string | いいえ | 編集に使用するセッション ID。既定値: ソースクリップのセッション。 |
| `strength` | number | いいえ | "cover" でのソースとの類似度。0〜1（既定値 0.5）。 |
| `title` | string | いいえ | 新しいクリップのタイトル。既定値: ソースのタイトルの後に括弧付きのアクション名。 |

### `music_stems`

曲をボーカル、ドラム、ベース、その他に分けるステム分離をキューに入れます。ジョブは 4 つのステムをダウンロードします。

| パラメーター | 型 | 必須 | 説明 |
| :--- | :--- | :--- | :--- |
| `clip_id` | string | はい | ソースクリップ ID。 |
| `download` | boolean | いいえ | 4 つのステム（ボーカル、ドラム、ベース、その他）をダウンロードします（既定値 true）。 |
| `format` | string | いいえ | ダウンロード形式: "m4a"（既定値）、"wav"、または "mp3"。 |
| `session_id` | string | いいえ | ジョブに使用するセッション ID。既定値: ソースクリップのセッション。 |

### `music_chat`

Google Flow Music の Producer エージェントへのメッセージをキューに入れます。Producer は歌詞の作成、曲の生成と編集、ステムの分離、Spaces の構築、ミュージックビデオの企画を自分で行えるため、メッセージがクレジットを消費する場合があります。会話を続けるには、セッション ID を使用します。

| パラメーター | 型 | 必須 | 説明 |
| :--- | :--- | :--- | :--- |
| `download` | boolean | いいえ | 返信で作成された新しいクリップのオーディオをダウンロードします（既定値 true）。 |
| `format` | string | いいえ | ダウンロード形式: "m4a"（既定値）、"wav"、または "mp3"。 |
| `message` | string | はい | Producer エージェントへのメッセージ。 |
| `session_id` | string | いいえ | 既存のセッション ID。省略すると新しいセッションを作成します。 |
| `timeout_secs` | integer | いいえ | 返信の最大待機時間（秒）（既定値 900、範囲 30〜3600）。 |

### `music_video`

ミュージックビデオの一覧表示、取得、状態確認、削除を行います。アクション "create" は曲の新しい動画をキューに入れます（クレジットを消費します）。種類 "music" は Producer に完全なミュージックビデオを依頼し、種類 "social" は短い縦型クリップを作成します。

| パラメーター | 型 | 必須 | 説明 |
| :--- | :--- | :--- | :--- |
| `action` | string | いいえ | アクション: "list"（既定値）、"get"、"status"、"delete"、または "create"（キューに入り、クレジットを消費します）。 |
| `clip_id` | string | いいえ | "create" に使用する曲のクリップ ID。 |
| `id` | string | いいえ | "get"、"status"、"delete" に使用するミュージックビデオ ID。 |
| `instructions` | string | いいえ | "music" 動画への追加の指示。例: 映像のスタイル。 |
| `kind` | string | いいえ | "create" の動画の種類: "music"（既定値。Producer が完全なミュージックビデオを企画します）または "social"（短い縦型クリップ）。 |
| `length_s` | integer | いいえ | "social" 動画の長さ（秒）（既定値 15）。 |
| `limit` | integer | いいえ | "list" で返す動画の最大数（既定値 20）。 |
| `offset` | integer | いいえ | "list" でスキップする動画の数（既定値 0）。 |
| `session_id` | string | いいえ | "create" に使用する既存のセッション ID。省略すると新しいセッションを作成します。 |

## キュー制御

### `music_wait`

キュー内または実行中の Google Flow Music ジョブの完了を待ちます。

| パラメーター | 型 | 必須 | 説明 |
| :--- | :--- | :--- | :--- |
| `id` | string | いいえ | `ids` を省略した場合に待機する単一のジョブ ID。 |
| `ids` | array of string | いいえ | 待機するジョブ ID（省略するとすべてのジョブを待機します）。 |
| `timeout` | integer | いいえ | 最大待機時間（秒）（既定値 600）。1 回の呼び出しは最大 45 秒後に `pending: true` を返します。 |

### `music_cancel`

キュー内または実行中の Google Flow Music ジョブをキャンセルします。キュー内のジョブは credits を消費しません。実行中のジョブでは、ツール呼び出しがすでに開始した clip をサーバーがレンダリングし続け、その credits を消費します。キャンセルで停止するのは、待機、ダウンロード、セッションです。

| パラメーター | 型 | 必須 | 説明 |
| :--- | :--- | :--- | :--- |
| `id` | string | はい | キューの 8 文字のジョブ ID。 |

### `music_retry`

失敗またはキャンセルされた Google Flow Music ジョブを再実行します。再度クレジットを消費します。

| パラメーター | 型 | 必須 | 説明 |
| :--- | :--- | :--- | :--- |
| `id` | string | はい | キューの 8 文字のジョブ ID。 |

## ライブラリ

### `music_songs`

Google Flow Music ライブラリ内の曲（クリップ）の一覧表示、取得、名前変更、公開設定の変更、削除を行います。

| パラメーター | 型 | 必須 | 説明 |
| :--- | :--- | :--- | :--- |
| `action` | string | いいえ | アクション: "list"（既定値）、"favorites"、"get"、"rename"、"privacy"、または "delete"。 |
| `ids` | array of string | いいえ | "get" と "delete" に使用するクリップ ID、または "rename" と "privacy" に使用する 1 つのクリップ ID。 |
| `include_disliked` | boolean | いいえ | "list" で低評価の曲を含めます（既定値 false）。 |
| `limit` | integer | いいえ | "list" と "favorites" で返す曲の最大数（既定値 20、最大 100）。 |
| `offset` | integer | いいえ | "list" と "favorites" でスキップする曲の数（既定値 0）。 |
| `privacy` | string | いいえ | "privacy" の新しい公開設定: "public"、"unlisted"、または "private"。 |
| `title` | string | いいえ | "rename" の新しいタイトル。 |

### `music_sessions`

Google Flow Music のセッション（Producer との会話）の一覧表示、トランスクリプトの読み取り、作成、名前変更、停止、削除を行います。

| パラメーター | 型 | 必須 | 説明 |
| :--- | :--- | :--- | :--- |
| `action` | string | いいえ | アクション: "list"（既定値）、"get"、"create"、"rename"、"stop"、または "delete"。 |
| `delete_songs` | boolean | いいえ | "delete" でセッションの曲も削除します（既定値 false）。 |
| `delete_spaces` | boolean | いいえ | "delete" でセッションの Spaces も削除します（既定値 false）。 |
| `delete_videos` | boolean | いいえ | "delete" でセッションのミュージックビデオも削除します（既定値 false）。 |
| `id` | string | いいえ | "get"、"rename"、"stop"、"delete" に使用するセッション ID。 |
| `limit` | integer | いいえ | "list" で返すセッションの最大数（既定値 30、最大 100）。 |
| `offset` | integer | いいえ | "list" でスキップするセッションの数（既定値 0）。 |
| `title` | string | いいえ | "rename" の新しいタイトル。 |

### `music_playlists`

プレイリストの一覧表示、取得、作成、更新、削除を行い、プレイリスト内の曲の追加、削除、並べ替えを行います。

| パラメーター | 型 | 必須 | 説明 |
| :--- | :--- | :--- | :--- |
| `action` | string | いいえ | アクション: "list"（既定値）、"favorites"、"get"、"create"、"update"、"delete"、"add"、"remove"、または "reorder"。 |
| `clip_ids` | array of string | いいえ | "add" と "remove" に使用するクリップ ID、または "reorder" の新しい完全な順序。 |
| `description` | string | いいえ | "create" と "update" に使用するプレイリストの説明。 |
| `id` | string | いいえ | "list"、"favorites"、"create" 以外のすべてのアクションに使用するプレイリスト ID。 |
| `limit` | integer | いいえ | "list" で返すプレイリストの最大数（既定値 20、最大 100）。 |
| `name` | string | いいえ | "create" と "update" に使用するプレイリスト名。 |
| `offset` | integer | いいえ | "list" でスキップするプレイリストの数（既定値 0）。 |

### `music_spaces`

Spaces（インタラクティブな楽器とツール）の一覧表示、取得、公開設定の変更、削除を行います。Space を作成するには、music_chat で Producer に内容を説明してください。

| パラメーター | 型 | 必須 | 説明 |
| :--- | :--- | :--- | :--- |
| `action` | string | いいえ | アクション: "list"（既定値）、"get"、"privacy"、または "delete"。Space は music_chat で作成します。 |
| `id` | string | いいえ | "get"、"privacy"、"delete" に使用する Space のバージョン ID（"list" の "id" フィールド）。 |
| `include_files` | boolean | いいえ | 各 Space のソースファイルを含めます（既定値 false）。 |
| `limit` | integer | いいえ | "list" で返す Space の最大数（既定値 20、最大 100）。 |
| `offset` | integer | いいえ | "list" でスキップする Space の数（既定値 0）。 |
| `privacy` | string | いいえ | "privacy" の新しい公開設定: "public"、"unlisted"、または "private"。 |

### `music_upload`

ローカルのオーディオファイル（music_edit と music_stems で使用できるクリップになります）または画像ファイル（music_generate 用の画像 ID を返します）をアップロードします。

| パラメーター | 型 | 必須 | 説明 |
| :--- | :--- | :--- | :--- |
| `kind` | string | いいえ | ファイルの種類: "audio" または "image"。既定値: ファイル形式から判定。 |
| `path` | string | はい | ローカルのオーディオファイル（4 分未満かつ 40 MiB 未満）または画像ファイル（5 MiB 未満）の絶対パス。 |

## ファイル

### `music_download`

曲のオーディオ（m4a、wav、または mp3）、動画、カバーアート、またはミュージックビデオを、ローカルのダウンロードディレクトリにダウンロードします。

| パラメーター | 型 | 必須 | 説明 |
| :--- | :--- | :--- | :--- |
| `format` | string | いいえ | "audio" のオーディオ形式: "m4a"（既定値）、"wav"、または "mp3"。 |
| `id` | string | はい | "audio" と "image" に使用するクリップ ID、または "video" に使用するクリップ ID かミュージックビデオ ID。 |
| `kind` | string | いいえ | ダウンロード対象: "audio"（既定値）、"video"、または "image"（カバーアート）。 |

### `music_downloads`

ローカルにダウンロードしたファイルの一覧表示、詳細確認、バイナリチャンクの読み取り、整理、削除、全消去を行います。

| パラメーター | 型 | 必須 | 説明 |
| :--- | :--- | :--- | :--- |
| `action` | string | いいえ | アクション名: "list"（既定値）、"get"、"read_chunk"、"delete"、"prune"、または "clear"。 |
| `asset_id` | string | いいえ | "get"、"read_chunk"、または "delete" に使用する Google Flow Music のクリップ ID または動画 ID。 |
| `existing_only` | boolean | いいえ | "list" で、ファイルが現在ディスク上に存在する行のみを返します（既定値 false）。 |
| `id` | integer | いいえ | "get"、"read_chunk"、または "delete" に使用するダウンロードレコードの行 ID。 |
| `kind` | string | いいえ | "list" のファイル種類フィルター: "audio"、"stem"、"video"、"image"、または "all"（既定値）。 |
| `length` | integer | いいえ | "read_chunk" の最大チャンクサイズ（バイト）（1..=4_194_304、既定値 1_048_576）。 |
| `limit` | integer | いいえ | "list" で返すダウンロードレコードの最大数（1..=500、既定値 50）。 |
| `offset` | integer | いいえ | "read_chunk" でのダウンロード済みファイル内のバイトオフセット（既定値 0）。 |
| `older_than_days` | integer | いいえ | アクションが "clear" の場合、この日数より前にダウンロードしたファイルのみを削除します。 |
| `query` | string | いいえ | "list" で、アセット ID、名前、プロンプト、ファイル名、またはパスに一致する、大文字と小文字を区別しない部分文字列。 |
| `session_id` | string | いいえ | "list" で、Google Flow Music のセッション ID によって絞り込みます。 |
