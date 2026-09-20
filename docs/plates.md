# プレートAPI

プレートは再利用するモデルの構成です。名前と、モデルの参照・個数を保存します。
SCADモデルのSTLは印刷準備の開始時に取得します。直接アップロードしたSTLはSQLite内に保管します。
起動方法は[README](../README.md)、印刷条件の指定は[キューAPI](queue.md)を参照してください。

## 保存と検索

手元の`box.stl`を登録します。複数のSTLは`models`フィールドを繰り返して送ります。

```sh
curl --fail http://127.0.0.1:3000/api/plates \
  -F 'name=Desk box' -F 'models=@box.stl'
curl --fail --get http://127.0.0.1:3000/api/plates --data-urlencode 'q=dsbx'
```

新規保存は201とプレートJSONを返します。`id`は固定の識別子、`version`は編集の競合を防ぐ整数です。
`models`には各項目の`id`、`name`、`source`、`quantity`を返します。
`source`はSCADの相対パス、直接アップロードでは`null`です。アップロード直後の個数は1です。
工程・材料設定・生成3MF・過去のrevisionはプレートに含みません。

`q`は名前とモデル名を大文字・小文字を区別せず部分列で絞り込みます。
連続一致・単語の先頭・ファイル名の一致を優先し、同順位は表示名、IDの順に並びます。

| 操作 | HTTP |
| --- | --- |
| 一覧・検索 | `GET /api/plates?q=...` |
| STLをアップロードして新規作成 | `POST /api/plates`（multipart） |
| SCAD参照で新規作成 | `POST /api/plates/import`（JSON） |
| プレート取得 | `GET /api/plates/{id}` |
| 構成を編集 | `PUT /api/plates/{id}`（JSON） |
| アップロード元のSTL取得 | `GET /api/plates/{id}/files/{model_id}` |

## SCADモデルを選ぶ

`SCAD_LIVE_URL`へ、OrcaServerから到達できるscad-liveのHTTP URLを指定します。
Dockerネットワーク上なら`http://scad-live:5003/`のようにサービス名を使えます。
URL内の資格情報・query・fragment、リダイレクトは受け付けません。
接頭パスがあれば、その末尾に`api/models`と`models/`を付けてアクセスします。
接続先は一覧に相対パスのJSON配列、モデル取得にSTL本体を返す必要があります。

```sh
curl --fail http://127.0.0.1:3000/api/scad/models
curl --fail http://127.0.0.1:3000/api/plates/import \
  -H 'Content-Type: application/json' \
  -d '{"name":"Desk parts","models":[{"name":"Box","source":"parts/box.stl","quantity":2}]}'
```

一覧には`q`で部分列検索を指定できます。`source`には一覧の相対パスをそのまま渡します。
空白・日本語・`%`のURL符号化はサーバーが行います。
保存時は参照と個数だけを登録し、STL取得やスライスは行いません。
参照先が印刷時に消失・取得失敗した場合は要確認となり、以前のSTLで代用しません。
一覧未設定は503、上流の通信・一覧異常は502です。各上流要求は15秒、一覧は1 MiBまでです。

## 構成を編集する

取得した`version`と、保存したい全項目を送ります。以下のIDと版は実際の応答に置き換えてください。

```json
{
  "version": 1,
  "name": "Desk parts",
  "models": [
    {"id": "既存のモデルID", "name": "Box", "source": "parts/box.stl", "quantity": 3},
    {"name": "Holder", "source": "parts/holder.stl", "quantity": 1}
  ]
}
```

既存の項目を残す場合はそのID、新しいSCAD参照はIDなしで指定します。省いた項目は構成から外れます。
アップロードした項目は、そのプレート内のIDと`source: null`を指定して保持します。
編集APIから別プレートのアップロードデータを参照したり、STL本体を差し替えたりはできません。
保存成功は200と更新後のJSON、古い版での保存は409です。読み直して構成を確認してください。
待機ジョブは準備開始時の構成を使います。既に準備を始めたジョブの入力は変更しません。

## 制限と保存先

名前は制御文字を含まない1〜256バイト、モデルは1〜64項目、各個数は1〜64、合計64個までです。
アップロード全体と準備時のSTL合計は64 MiBまでです。ASCII・binary STLを扱い、空のモデルや非有限座標を拒否します。
モデルの参照には`..`・絶対パス・バックスラッシュを使えません。
未知のID・未登録ファイルは404、入力不正は400または422です。
書込みは同じOriginだけを受け付け、別サイトは403です。プロキシでは利用者側のHostを維持してください。

`<PLATES_DIR>/orca.sqlite3`の`plates`と`plate_items`へ保存します。
直接アップロードしたSTL本体も同じDBのBLOBに含まれるため、DBから復元できます。
SCADの元データはscad-live側で保管・バックアップします。
保存領域はアプリ専用にし、一つの保存先を複数サーバーで共有しないでください。
旧ファイル形式からの移行とバックアップは[コンテナガイド](container.md#更新とバックアップ)を参照してください。

検索順位は[scad-live](https://github.com/miyabi-sunny-side/scad-live)の方式を基にしています。
MITの著作権・許諾表示は[第三者通知](../THIRD_PARTY_NOTICES)に保持しています。
