# プレートAPI

STLファイルをまとめてプレートへ保存し、名前やモデルのパスで検索できます。
サーバーの起動方法は[README](../README.md)を参照してください。

## 保存と検索

手元の`box.stl`を登録します。複数のSTLは`models`フィールドを繰り返して送ります。

```sh
curl --fail http://127.0.0.1:3000/api/plates \
  -F 'name=Desk box' -F 'models=@box.stl'
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

## 配置と印刷データの生成

保存後は[自動配置とスライス](slicing.md)のAPIで3MFを生成できます。
STLや設定の再保存・再取り込みは生成物を解除します。新しい内容で再スライスしてください。

`GET /api/plates/{id}/layout`は、保存した3MFの配置を次の形で返します。

```json
{"revision":"revision-id","models":[{"index":0,"bounds":[[118,138],[107,127],[0,20]]}]}
```

`index`はプレートJSONの`models`に対応する0始まりの番号です。
`bounds`は変換後のX・Y・Z座標の最小・最大値で、単位はmmです。形状の輪郭ではなく外形範囲を表します。
`revision`がプレートJSONと異なる場合は、プレートを読み直してから表示してください。
配置前は404を返します。

## scad-liveからの取り込み

OrcaServerの環境変数`SCAD_LIVE_URL`へ、サーバーから到達できるscad-liveのURLを指定します。
たとえば`http://scad-live:5003/`です。`scad-live`は例示用のホスト名なので、利用環境の値に変えてください。
Dockerでは`-e SCAD_LIVE_URL=http://scad-live:5003/`を起動引数へ加えます。
接続はブラウザからではなく、OrcaServerのプロセス・コンテナから行います。

この版はHTTP接続を扱います。URL内の資格情報・query・fragment、リダイレクトは受け付けません。
URLに`/scad/`などの接頭パスがある場合も、末尾に`api/models`と`models/`を付けてアクセスします。
接続先の一覧APIは相対パスのJSON配列、モデル取得APIはSTL本体を返す必要があります。

```sh
curl --fail http://127.0.0.1:3000/api/scad/models
curl --fail http://127.0.0.1:3000/api/plates/import \
  -H 'Content-Type: application/json' \
  -d '{"name":"Desk parts","models":["box one.stl","parts/holder.stl"]}'
```

一覧の`GET /api/scad/models`にも`q`を指定でき、同じ部分列検索でモデルのパスを絞り込めます。
`models`には一覧で返された相対パスを、そのまま文字列で渡します。
空白・日本語・`%`などのURL符号化はサーバーが行います。クライアント側で二重に符号化しないでください。
保存後にscad-live側が再生成されても、保存済みプレートは変わりません。
同じプレートへ最新版を取り込み直す場合は、要求JSONへ既存の`plate_id`を加えて送ります。
指定した全モデルを取り直して置換し、`settings`を省くと空の設定に戻します。
新規は201、再取り込みは200とプレートJSONを返します。

接続未設定は503、上流の通信・一覧・取得の異常は502、不正な選択やSTLは400です。
取得が1つでも失敗した場合は保存しません。再取り込みの失敗でも既存プレートを保持します。
各HTTP要求の期限は15秒、一覧は1 MiB、選択は1〜64モデル、取得内容の合計は64 MiBまでです。

## 制限

プレート名は制御文字を含まない1〜256バイト、STLは1〜64個です。
アップロード全体の上限はフォームのヘッダーを含め64 MiBです。
`settings`は16 KiB以下のJSONオブジェクトです。
STLはASCII・binaryの両形式を読み、空のモデルや有限でない座標を拒否します。
保存時には形状の印刷適性を判定しません。スライス時にOrcaSlicerが検査します。
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
直接アップロードしたモデルの`source`は`null`、scad-liveから取り込んだ場合は元の相対パスです。
配置・モデル・スライス設定を持つ`project`と派生印刷データの`print`は別の参照です。
スライス前は両方とも`null`です。スライス後は同じrevision内の`project.3mf`と`print.gcode.3mf`を参照します。

モデルを別revisionへ書き終えてから、`plate.json`を同じファイルシステム内で置き換えます。
保存失敗でも既存revisionを書き換えません。過去のrevisionと失敗時の未参照ファイルは自動削除しません。
バックアップはサービスを停止し、保存先のディレクトリ全体をコピーしてください。
壊れたメタデータはログへ記録して一覧から除外します。元データを自動削除・修復しません。
保存領域はアプリ専用とし、手動編集やシンボリックリンクの配置を避けてください。
コンテナへホストのフォルダーを渡す場合は、UID/GID 10001の書込み権限が必要です。

検索順位は[scad-live](https://github.com/miyabi-sunny-side/scad-live)の方式を基にしています。
MITの著作権・許諾表示は[THIRD_PARTY_NOTICES](../THIRD_PARTY_NOTICES)に保持しています。
