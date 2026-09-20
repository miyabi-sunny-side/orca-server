# プレートAPI

STLファイルをまとめてプレートへ保存し、名前やモデルのパスで検索できます。
サーバーの起動方法は[README](../README.md)を参照してください。

## 保存と検索

手元の`box.stl`を登録します。複数のSTLは`models`フィールドを繰り返して送ります。

```sh
curl --fail http://127.0.0.1:3000/api/plates \
  -F 'name=Desk box' -F 'models=@box.stl' -F 'settings={"material":"PLA"}'
curl --fail --get http://127.0.0.1:3000/api/plates --data-urlencode 'q=dsbx'
```

保存時は201とプレートのJSONを返します。`id`は固定の識別子で、表示名とは別です。
`q`を省くと全件、指定すると大文字・小文字を区別せず部分列で絞り込みます。
検索は名前と各モデルのパスが対象です。連続一致・単語の先頭・ファイル名の一致を優先します。
順位はサーバーが決め、同順位は表示名、IDの順に並びます。

| 操作 | HTTP |
| --- | --- |
| 一覧・検索 | `GET /api/plates?q=...` |
| 新規保存 | `POST /api/plates` |
| プレート取得 | `GET /api/plates/{id}` |
| 内容の全置換 | `PUT /api/plates/{id}` |
| モデル取得 | `GET /api/plates/{id}/files/{models[].path}` |

全置換も同じmultipart形式です。`name`と全ての`models`を渡してください。
`settings`は省略すると空のJSONオブジェクトになります。同じIDへの同時保存は最後の成功分が有効です。
ファイル取得には、プレートJSONの`models[].path`をURLの末尾へ使います。
未登録のファイルや未知のIDは404、不正な入力は400、保存領域の障害は500です。
ブラウザからの書込みは同じホスト・ポートを持つOriginだけを受け付け、別サイトは403で拒否します。
プロキシを使う場合は、利用者側のHostヘッダーを維持してください。

## 制限

プレート名は制御文字を含まない1〜256バイト、STLは1〜64個です。
アップロード全体の上限はフォームのヘッダーを含め64 MiBです。
`settings`は16 KiB以下のJSONオブジェクトです。
STLはASCII・binaryの両形式を読み、空のモデルや有限でない座標を拒否します。
形状の印刷適性や設定値のスライサー互換性は判定しません。
モデル名は相対パスを使えますが、`..`・絶対パス・バックスラッシュは拒否します。
アップロードされた名前を保存先のファイル名には使いません。

## 保存形式と保全

`PLATES_DIR`配下の構成は次のとおりです。UUIDはサーバーが生成します。

```text
<plate-id>/
  plate.json
  revisions/<revision-id>/0.stl
  revisions/<revision-id>/1.stl
```

`plate.json`は形式版`format_version: 1`とID・revision・表示名・モデル一覧・設定を持ちます。
モデルには表示用の`name`、保存先の相対`path`、取り込み元の`source`があります。
直接アップロードしたモデルの`source`は`null`です。
配置・モデル・スライス設定を持つ`project`と派生印刷データの`print`は別の参照です。
この版は3MFを生成せず、両方とも`null`を返します。

モデルを別revisionへ書き終えてから、`plate.json`を同じファイルシステム内で置き換えます。
保存失敗でも既存revisionを書き換えません。過去のrevisionと失敗時の未参照ファイルは自動削除しません。
バックアップはサービスを停止し、保存先のディレクトリ全体をコピーしてください。
壊れたメタデータはログへ記録して一覧から除外します。元データを自動削除・修復しません。
保存領域はアプリ専用とし、手動編集やシンボリックリンクの配置を避けてください。
コンテナへホストのフォルダーを渡す場合は、UID/GID 10001の書込み権限が必要です。

検索順位は[scad-live](https://github.com/miyabi-sunny-side/scad-live)の方式を基にしています。
MITの著作権表示は[LICENSE](../LICENSE)に保持しています。
