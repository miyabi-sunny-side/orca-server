# 自動配置とスライス

保存済みのSTLをOrcaSlicerで配置し、編集用`project.3mf`と印刷用`print.gcode.3mf`を生成します。
対象はP1Sの0.4mmノズル、1プレート、単一材料です。プリンターへの送信は行いません。

## OrcaSlicerの設定

Linuxでソースから起動する場合に利用できます。現在の公開コンテナにはOrcaSlicerを含みません。
[公式v2.4.2](https://github.com/OrcaSlicer/OrcaSlicer/releases/tag/v2.4.2)の、実行環境に合うAppImageを取得します。
Ubuntu 24.04向けx86_64版では、次のように展開します。

```sh
chmod +x OrcaSlicer_Linux_AppImage_Ubuntu2404_V2.4.2.AppImage
./OrcaSlicer_Linux_AppImage_Ubuntu2404_V2.4.2.AppImage --appimage-extract
export ORCA_APPDIR="$PWD/squashfs-root"
"$ORCA_APPDIR/AppRun" --help
```

ホストにGTK 3、WebKitGTK 4.1、GStreamer、OpenGLの実行ライブラリが必要です。
OpenGLの不足を公式ランチャーが報告した場合は、
Ubuntuでは`libopengl0`と`libglu1-mesa`、Arch系では`libglvnd`を導入します。
GUIセッションは不要です。版を変更したバイナリや別の版のprofileを混ぜないでください。

同じシェルで[開発ガイド](development.md#ソースから起動)のサーバーを起動します。
`ORCA_APPDIR`は`AppRun`と`resources`があるディレクトリの絶対パスです。
起動時にCLIの版と既定profileを確認します。未設定ならスライスAPIは503を返し、保存・検索は使えます。
`ORCA_TIMEOUT_SECS`は各CLI工程の上限秒数です。既定300、設定範囲1〜3600です。

## 設定を選んで実行

[プレートAPI](plates.md)でSTLを保存し、返された`id`を使います。
利用できる設定名は次のAPIから取得します。

```sh
curl --fail http://127.0.0.1:3000/api/slicer/profiles
```

応答は`printer`、`processes`、`filaments`、`beds`、`defaults`とCLIの`version`です。
工程と材料は、配布物のBBL profileでP1S 0.4mmに対応するものを列挙します。
プレートの保存時に、`settings`の`slicer`へ選択した名前を指定します。
省略した項目には次の既定値を使います。

```json
{
  "slicer": {
    "process": "0.20mm Standard @BBL X1C",
    "filament": "Generic PLA High Speed @BBL X1C",
    "bed": "Textured PEI Plate"
  }
}
```

```sh
curl --fail -X POST http://127.0.0.1:3000/api/plates/PLATE_ID/slice
```

`PLATE_ID`を保存時のIDに置き換えます。応答は処理完了後のプレートJSONです。
`project`と`print`に生成物のパス、`settings.slicer`に適用した設定を返します。
`GET /api/plates/PLATE_ID/files/返されたパス`でダウンロードできます。
保存したprojectだけを入力してスライスするため、配置・設定は両方の3MFに保持されます。
STLや設定を保存し直した場合は、再度スライスしてください。

## 失敗時の挙動

同時に実行できるのは1件です。実行中の追加要求は409を返します。
処理中に対象プレートを編集した場合も409となり、新しい編集を上書きしません。
不正設定は400、CLIの失敗は502、時間超過は504です。
1プレートに収まらない場合は、CLIまたは生成物の検査で拒否し、502または400を返します。
時間超過では子プロセスを終了します。CLIの終了・出力欠落・モデル欠落を成功扱いにしません。
配置・スライスの両方が成功した場合だけ、新しいrevisionを保存します。
失敗時は既存のSTLと生成物を保持し、一時ディレクトリは破棄します。

進行中の工程、CLIの終了コード、標準出力・標準エラーはサーバーログで確認できます。
CLI出力は各ストリーム512 KiBまで記録し、それ以降は抑制します。
画面描画なしではサムネイルPNGを生成できない場合があります。PNGは成功条件に含めません。

## profileと保存形式

実行バイナリと同じ配布物の`resources/profiles/BBL`を読みます。
サブディレクトリを含むprofileを名前で参照し、`inherits`の親から子の順に設定を上書きします。
配列も子の値で置き換え、展開後は`inherits`を除きます。親の欠落や循環は拒否します。
任意のファイルパス、CLI引数、G-codeの上書きをAPIから渡す機能はありません。

生成物は`<PLATES_DIR>/<id>/revisions/<revision>/`にSTLと一緒に保存します。
現在のrevisionは`plate.json`が参照します。1つの保存先を複数サーバープロセスで共有しないでください。

## CLI連携の検証

公式AppImageを展開したLinux環境で実行します。プリンターへは接続しません。

```sh
cargo build --locked
python3 tests/slicer_cli.py "$ORCA_APPDIR" /tmp/orca-cli-check
```

2個の立方体の実座標、設定の保持、配置不能、CLI失敗・時間超過と既存データの保全を確認します。
結果・3MF・サーバーログは指定した出力先へ保存します。時間超過と終了失敗は隔離したランチャーで起こします。
通常の配置とスライスには公式バイナリを使います。
