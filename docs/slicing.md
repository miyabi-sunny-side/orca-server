# 印刷準備とスライス

キューの開始・継続ボタンで準備を開始すると、モデルをOrcaSlicerで配置し、編集用`project.3mf`と印刷用`print.gcode.3mf`を生成します。
同梱BBLの単一ノズル構成、1プレート・単一材料が対象です。生成後は同じ印刷データをFTPSで送信します。
プレートの保存だけでは取得・配置・スライスを実行しません。

## OrcaSlicerの設定

公開コンテナには設定済みのOrcaSlicer 2.4.2を含みます。以下はLinuxでソースから起動する場合の手順です。
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
起動時にCLIの版と既定profileを確認します。未設定でも保存・検索は使えますが、印刷準備は開始できません。
`ORCA_TIMEOUT_SECS`は各CLI工程の上限秒数です。既定300、設定範囲1〜3600です。

## 設定と生成物

機種・ノズルと工程・ビルドプレートは[プレート](plates.md#印刷条件を保存する)で指定します。
材料の基本プロファイルと温度の上書きは[材料設定](filaments.md)に保存します。
候補は`GET /api/slicer/profiles?machine=URLエンコードしたmachine_profile_key`で取得できます。
応答は`printer`、`processes`、`filaments`、`beds`、`defaults`、CLIの`version`です。
機種を省略するとP1S 0.4mm用です。未知・消失したプロファイルは400で拒否し、別機種へ切り替えません。

準備開始時にプレート構成と現在の設定を固定し、SCAD参照から最新STLを取得します。
指定した個数分を配置し、配置済みprojectだけを入力してスライスします。
生成物のモデル数・座標・単一プレート・材料と設定を検査してから、印刷用3MFを転送します。
生成物は`<PLATES_DIR>/jobs/<job-id>/<attempt-id>/`に固定入力とともに保存します。
再利用プレートの編集や、別ジョブの準備でこの実行のデータを書き換えません。
取り外し完了・取消後に片付けるため、恒久的な生成物ライブラリや履歴としては扱いません。

## 失敗時の挙動

CLIはサーバー内で1件ずつ実行し、他の準備は順番を待ちます。
STL取得失敗・配置不能・不正設定・CLI失敗・時間超過はキューを要確認にし、開始命令を送信しません。
以前のSTLや別設定へ切り替えて成功扱いにすることはありません。再利用プレートの構成は維持します。
時間超過では子プロセスを終了します。出力欠落やモデル欠落も失敗です。
理由と本体を確認して[キューから復旧](queue.md#失敗再起動からの復旧)してください。

工程・CLI終了コード・標準出力と標準エラーはサーバーログで確認できます。
出力は各ストリーム512 KiBまで記録します。ヘッドレス実行ではPNGを生成できない場合がありますが、印刷の成功条件には含めません。

## プロファイル

実行バイナリと同じ配布物の`resources/profiles/BBL`を読みます。
`inherits`の親から子へ設定を展開し、配列も子の値で置き換えます。親の欠落・循環は拒否します。
展開した基本設定に、DBに保存したノズル・ベッド温度の明示的な上書きだけを適用します。
任意のファイルパス・CLI引数・G-codeをAPIから指定する機能はありません。

## CLI連携の検証

公式AppImageを展開したLinux環境で実行します。

```sh
cargo build --locked
python3 tests/slicer_cli.py "$ORCA_APPDIR" /tmp/orca-cli-check
```

P1S 0.4mmとA1 mini 0.2mmのプロファイルで、2個のモデルの配置・座標・温度設定・印刷データを検査します。
配置不能時の開始拒否も確認します。MQTT/FTPSは隔離したテスト用の接続先で、実機へは接続しません。
CLI終了失敗・時間超過はRustの隔離ランチャーテストで確認します。
結果・3MF・ログを指定先へ保存します。これらは物理的な印刷品質や他機種の実通信を保証する検証ではありません。
