# MCPでプレートと材料を管理する

Streamable HTTP対応のMCPクライアントから、プレートの保存・更新、製品と色、共通温度、AMSの対応と使用順を操作できます。
ブラウザと同じAPI・SQLiteを使います。MCPで印刷キューの追加や印刷開始は行いません。

## 接続

起動済みOrcaServerの`http://127.0.0.1:3000/mcp`を、クライアントのStreamable HTTP接続先に設定します。
別端末から使う場合はホスト名・ポートをサーバーのものに置き換えます。接続範囲は信頼できるLAN/Tailscaleに制限してください。
追加のMCPプロセスやトークン設定は不要です。ブラウザからの別originの書込みは拒否します。

SCAD参照の作成・更新には、サーバーの[SCAD_LIVE_URL](container.md#起動と接続)を設定します。
材料の機種別設定・工程候補には同梱OrcaSlicerが必要です。公式コンテナには含まれます。

クライアントで接続し、`tools/list`を取得します。各toolの`inputSchema`が引数のJSON Schemaです。
通常のクライアントは接続時の初期化とtool一覧取得を行います。
本書のJSON例は`tools/call`の`arguments`に渡す値です。

成功時の`structuredContent`は`{"data":...}`です。プレートの保存・取得は`ui_path`も返し、
接続したサーバーのブラウザURLに付けると詳細画面を開けます。`content`にも同じJSONを返します。

## モデル10個をプレートに保存する

まず`scad_models`で公開済みSTLを検索します。

```json
{"q":"gridfinity"}
```

返された相対パスを使って`plate_save`を呼びます。次の`Gridfinity/bin.stl`は実際の検索結果に置き換えてください。

```json
{
  "plate": {
    "name": "Gridfinity bins ×10",
    "models": [
      {"name":"Gridfinity/bin.stl","source":"Gridfinity/bin.stl","quantity":10}
    ],
    "conditions": {
      "required_machine_profile_key":null,
      "filament_id":null,
      "process_profile_key":null,
      "bed_type":null
    }
  }
}
```

保存結果の`data`には`id`、`version`、`name`、`models`、`conditions`が入ります。
`ui_path`は`/plates/保存されたID`です。モデルの個数合計は1〜64個です。
保存時はscad-liveの一覧で存在を確認し、STL本体は印刷準備時に取得します。SCAD自体の公開はscad-live側で行ってください。

更新する場合は`plate_get`に`{"id":"取得したプレートID"}`を渡し、現在の構成・版を読みます。
`plate_save`の外側に同じ`id`を指定し、`plate.version`と保存する全モデル・全条件を送ります。
既存モデルの`id`を保持してください。アップロード済みSTLはそのプレート内のモデルIDと`source:null`で保持できます。
MCPからファイル本体をアップロードする操作はありません。

`conditions`を省略すると4項目とも未設定になります。機種が未設定なら工程も未設定にします。
条件を選ぶ際は`plate_options`の所持機候補、製品の色ID、機種別の共通設定を使います。
新規作成の省略・`null`には、サーバーが[保存済みの初期値とAMS材料](plates.md#新規作成の初期値)を適用します。明示した有効値を優先します。
`profiles.defaults`からAIが条件を推測する必要はありません。保存応答の`conditions`を確認してください。
更新では省略・`null`が解除となるため、保持する条件も送ります。
保存成功と印刷可能は別です。`plate_admission`で、選んだ実機に追加できるかと理由を確認できます。

## 製品に色と共通温度を追加する

`filament_products`で既存製品と色・設定を読み、対象の製品IDを選びます。
`filament_color_save`なら、共通温度を入力し直さずに黄色を追加できます。

```json
{"product_id":"取得した製品ID","data":{"name":"Yellow","color":"FFFF00FF"}}
```

返された色の`data.id`を、プレートやAMSの`filament_id`に使います。製品IDとは異なります。
色の更新は同じ引数に色の`id`を追加します。色名から別のRGBAへ置き換えず、正確な8桁の値を保持します。

温度は`filament_profiles`で機種に対応する基本プロファイルを調べ、`filament_setting_save`で製品へ保存します。
既存設定の更新には製品詳細から取得した設定IDを外側の`id`へ指定します。

```json
{
  "product_id":"取得した製品ID",
  "id":"取得した設定ID",
  "data": {
    "machine_profile_key":"取得した機種・ノズルキー",
    "base_profile_key":"互換候補から選んだキー",
    "overrides_json":{"nozzle_temperature":218}
  }
}
```

未指定の温度差分は基本値に戻り、その製品の全色へ反映します。
値の範囲、AMSの観測と台帳の違いは[材料管理](filaments.md)を参照してください。

## Toolと引数

`id`は各読取り・保存結果から取得します。「任意」は省略可能で、それ以外は必須です。
`data`内の型・未許可キーも検証します。

| Tool | 引数と内容 |
| --- | --- |
| `scad_models` | 任意の`q: string`。公開済みモデルの部分列検索。 |
| `plate_list` | 任意の`q: string`。保存済みプレートの検索。 |
| `plate_get` | `id: string`。詳細・版・管理画面パス。 |
| `plate_save` | 任意の`id: string`と`plate`。新規はID省略、更新は現在版を含む全構成。形式は[プレートAPI](plates.md)。 |
| `filament_products` | 引数なし。製品・色・共通設定一覧。 |
| `filament_product` | `id: string`。製品詳細と解決済み温度。 |
| `filament_product_save` | 任意の`id`と`data: {name,vendor,material,bambu_filament_id}`。最後の項目は文字列またはnull。 |
| `filament_color_save` | `product_id`、任意の色`id`、`data: {name,color}`。RGBA色の追加・更新。 |
| `filament_setting_save` | `product_id`、任意の設定`id`、`data: {machine_profile_key,base_profile_key,overrides_json}`。共通設定の追加・更新。 |
| `filament_profiles` | `product_id`、`machine`。材料と機種に対応する基本プロファイル。 |
| `filaments_search` | 任意の`q`。製品・メーカー・材料・色から色IDを検索。 |
| `printers` | 引数なし。登録実機と報告状態。 |
| `ams_get` | `printer_id`。slot ID・revision・観測・材料対応・使用順。 |
| `ams_assign` | `printer_id`、`slot_id`、`revision: integer`、`filament_id: string or null`。色IDの割当/解除。最後の項目は省略不可。 |
| `ams_resolve` | `printer_id`、`filament_id`。現在使用できる候補と開始時の優先slot。 |
| `ams_prioritize` | `printer_id`、`priority: {filament_id,order:[{id,revision}]}`。現在の同一材料グループ全件を使用順に指定。 |
| `plate_options` | 任意の`machine`。所持機の機種/ノズル一覧と、指定機種の工程・材料・bed候補。 |
| `plate_admission` | `printer_id`、`plate_id`。現在版の`plate_version`、`allowed`、`reason`。 |

`plate_admission`は、条件の不足、実機との不一致、未同期や該当材料の未装填など、通常のキュー追加と同じ判断を返します。
機種候補は要求profile、`printer_id`は実物を識別します。同型機が複数あるときも対象を区別してください。
AMSの使用順は印刷開始時の順序です。機器の自動補充順や現在の印刷を変更する操作ではありません。

## エラーと結果不明時

tool実行時の拒否は`isError:true`と`structuredContent`の`status`・`error`に返します。
例えば古いプレート版は次の結果になります。

```json
{"status":409,"error":"Plate changed; reload before saving"}
```

400は不正な値・未知のモデル参照、404は存在しないID、409は古い版・revisionや現在状態との競合です。
JSON Schemaに合わない型や未許可の引数は、MCPの引数エラーまたはAPIの422です。
SCAD未設定などは503、上流の通信・応答異常は502です。
拒否された保存で既存の正常な構成やAMSの対応を部分更新しません。

通信断などで結果が分からない場合は、作成をそのまま再送しないでください。
`plate_list/get`や`filament_products/product`で名前・構成・色・設定を読み、保存されたか確認します。
AMSの操作も`ams_get/resolve`で読み直します。既存設定の更新は全置換なので、必要な値を保持して送ってください。

配布イメージと対応ソースは[リリース一覧](https://github.com/miyabi-sunny-side/orca-server/releases)から取得できます。
MCPの導入だけで保存形式は変わりません。データ更新と復旧の手順は[コンテナガイド](container.md)を参照してください。
