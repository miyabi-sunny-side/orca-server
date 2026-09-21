# 開発ガイド

## ソースから起動

Rust 1.96.0、Node.js 24、npmを用意します。Rustの版は`rust-toolchain.toml`で指定しています。

```sh
git clone https://github.com/miyabi-sunny-side/orca-server.git
cd orca-server
npm --prefix client ci
npm --prefix client run build
cargo run --locked
```

[ローカルの画面](http://127.0.0.1:3000)を開きます。Ctrl+Cで終了します。
別ターミナルで`npm --prefix client run dev`を実行すると、ポート5173で開発画面を表示します。
API要求はポート3000へ転送されます。配布する際は画面とRustを再ビルドしてください。

## 設定とAPI

| 環境変数 | 既定値 | 内容 |
| --- | --- | --- |
| `PORT` | `3000` | 待受ポート。1〜65535の整数。不正な値では起動しません。 |
| `LOG_LEVEL` | `info` | `off`、`error`、`warn`、`info`、`debug`、`trace`。不正な値は`info`です。 |
| `PLATES_DIR` | `data/plates` | プレートとSQLite台帳の保存先。コンテナ内では`/data/plates`。書込み権限が必要です。 |
| `SCAD_LIVE_URL` | 未設定 | scad-liveのHTTP URL。モデル一覧・SCAD参照の保存/更新・印刷時の取得に必要です。 |
| `ORCA_APPDIR` | ネイティブでは未設定、コンテナでは`/opt/orcaslicer` | 公式OrcaSlicer 2.4.2の展開先。詳細は[スライス](slicing.md)を参照。 |
| `ORCA_TIMEOUT_SECS` | `300` | 各CLI工程の上限秒数。OrcaSlicerを設定する場合は1〜3600。 |

ネイティブ実行では全IPv4インターフェースで待ち受けます。
到達範囲はホストのファイアウォールやコンテナのポート公開で制限します。
`GET /healthz`は`ok`、`GET /api/health`は`{"status":"ok"}`を返します。
プレートの操作と保存形式は[プレートAPI](plates.md)を参照してください。

複数プリンターの台帳・環境変数からの初回取り込み・状態取得・印刷APIは[プリンター接続](printer.md)を参照してください。
[材料台帳・機種別設定・AMS対応API](filaments.md)も利用できます。
[MCP](mcp.md)は同じプロセスの`/mcp`で提供し、既存APIの検証・競合制御を使います。
未設定でもプレートの保存・閲覧は使えます。印刷予定の操作は[キューAPI](queue.md)を参照してください。

## 検証

リポジトリのルートで実行します。ブラウザ検証にはChromiumを使用します。

```sh
npm --prefix client ci
npm --prefix client run format:check
npm --prefix client run check
npm --prefix client test
npx --prefix client playwright install chromium
npm --prefix client run test:e2e
npm --prefix client run build
npm --prefix client run lint:design
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
cargo build --locked --release
```

ブラウザテストは一時的なViteサーバーと隔離したAPI応答を使い、検索・失敗時の再試行・明暗・狭幅を確認します。
RustのテストはHTTP応答と組み込み画面、一時保存先での復元・入力拒否・失敗時の保全を確認します。
CIではビルドした実行ファイルを別ディレクトリから起動し、画面の配信も確認します。

公式OrcaSlicerを[設定](slicing.md#orcaslicerの設定)した環境では、実APIとCLIを通した画面操作も確認できます。
先に上記の手順で画面をビルドし、Chromiumを導入します。

```sh
cargo build --locked
python3 tests/browser_cli.py "$ORCA_APPDIR" /tmp/orca-browser-check
```

一時保存先とscad-live互換のHTTP応答を用意し、モデル選択・個数と条件の保存・直接追加・印刷・取り外しを操作します。
RustサーバーとOrcaSlicerは実際に実行します。スクリーンショットと操作結果は指定した出力先へ保存します。
MQTT/FTPSは隔離した接続先を使い、実プリンターには接続しません。起動したサーバーと一時データは終了時に片付けます。

MQTT接続の隔離検証にはPython 3と`openssl`コマンドを使います。
一時証明書のTLS接続先を用意し、購読・全状態要求・部分更新・再接続・証明書不一致・秘密値の非公開を確認します。
台帳のブラウザ検証には、上記のChromiumと`ORCA_APPDIR`の設定も必要です。

```sh
cargo build --locked
python3 tests/printer_mqtt.py target/debug/orca-server /tmp/orca-mqtt-check
python3 tests/printer_start.py target/debug/orca-server /tmp/orca-start-check
python3 tests/queue_printer.py target/debug/orca-server /tmp/orca-queue-check
python3 tests/mcp_printer.py target/debug/orca-server /tmp/orca-mcp-check
python3 tests/mcp_queue.py target/debug/orca-server /tmp/orca-mcp-queue
python3 tests/browser_queue.py target/debug/orca-server /tmp/orca-queue-browser
DEFAULTS_BROWSER=1 python3 tests/plate_defaults.py target/debug/orca-server /tmp/orca-defaults-check
PLATE_BROWSER=1 python3 tests/plate_queue.py target/debug/orca-server /tmp/orca-plate-check
REGISTRY_BROWSER=1 python3 tests/printer_registry.py target/debug/orca-server "$ORCA_APPDIR" /tmp/orca-registry-check
FILAMENT_BROWSER=1 python3 tests/filament_ams.py target/debug/orca-server "$ORCA_APPDIR" /tmp/orca-filament-check
```

印刷開始の検証はFTPSのTLSセッション再利用・転送内容・AMS指定・拒否・通信断・重複操作も確認します。
`tests/fixtures/p1_print.gcode.3mf`はOrcaSlicer 2.4.2で生成した通信検証用ファイルです。
生成元は同梱の20mm立方体STL 2個、プリンターはP1S 0.4mmです。
工程は0.20mm Standard、材料はGeneric PLA High Speed、プレートはTextured PEI Plateです。
初期値の検証ではSQLite移行・再起動・既定機選択、同期済みAMSの先頭、REST/MCPの一致、遅い取得と手動選択、印刷中の入力保全を確認します。
プレート条件の検証ではnullable保存、所持機からの候補選択、実機別の装填照合、直接追加を確認します。
キュー検証では準備時の最新データ固定、取り外し待ち、同時・重複操作、転送中AMS交換、開始前後の再起動とDB復元を通します。
`print_fixture.py`のCLI代替は入力・状態遷移の検証用です。実際の配置・スライスは`tests/slicer_cli.py`とコンテナ検証で公式Orcaを実行します。
CIでもMQTT・FTPS・キューの経路を検証します。実機や利用者のアクセスコードには接続しません。
MCP検証は実クライアントの接続・tool呼出しから、10個の構成、共通温度、色追加、AMS対応・使用順を確認します。
保存・材料操作では同じデータをREST APIで取得し、古い版・revisionの拒否と印刷命令が送られないことも検証します。
キューのMCP検証では隔離先への開始・継続・取り外し完了と、同一要求の再送・古い対象・接続断・材料保留での命令回数を観測します。

## 構成

- `src/`: Axumのルーター、起動処理、環境変数の読込み。
- `client/`: Svelte 5の画面と検証コード。
- `build.rs`: ビルド済み画面の存在確認と変更追跡。
- [DESIGN.md](../DESIGN.md): 画面・テーマ・操作の設計。
- `Dockerfile`: 画面とRustのビルド、非rootの実行イメージ。
- `.github/workflows/`: CIとタグによるコンテナ公開。

`client/dist`はコンパイル時に取り込まれ、実行時の静的ファイル配置は不要です。
未知の`/api/*`は404を返します。それ以外のURLには画面を返します。

## ビルドと公開

```sh
npm --prefix client run build
cargo build --locked --release
docker build -t orca-server .
```

`Cargo.toml`の版と一致する`vX.Y.Z`タグをpushすると、コンテナを公開します。
配布先は`ghcr.io/miyabi-sunny-side/orca-server`です。
GitHub Releaseには公開イメージのdigestとOrcaSlicerの依存ソースを添付します。
[コンテナガイド](container.md)に導入と保存先、[同梱ソース](container.md#ライセンスとソース)に版と取得方法を記載しています。

```sh
python3 tests/container.py orca-server /tmp/orca-container-check
docker build --target sources --output type=local,dest=/tmp/orca-sources .
```

コンテナ検証はLinuxのhostネットワークと専用の一時保存先を使います。非rootの配置・スライス、
再作成後のプレート・アップロードSTL・キューの保持、不明な開始の自動再送がないことを確認します。
`openssl`とPython 3、Dockerを使い、実プリンターへは接続しません。
`sources` targetはOrcaSlicer本体と24種の依存アーカイブ、固定commitのwxWidgetsとsubmoduleをまとめます。
Rust依存は`cargo vendor --locked`で取得し、別のソースアーカイブへまとめます。
OrcaSlicerのソースURLとSHA-256は`packaging/slicer-sources.txt`、構成元は上流2.4.2の`deps/*.cmake`です。
既に同梱された依存ソースは本体アーカイブに保持し、未使用のNanoSVG取得設定は含めません。

元のテンプレートは`fa63d25dbcb3762e2ecf7e56bfaddb994cdba07c`です。
MITの著作権・許諾表示は[THIRD_PARTY_NOTICES](../THIRD_PARTY_NOTICES)に保持しています。


### ライセンスと対応ソース

OrcaServerは[AGPL v3](../LICENSE)（`AGPL-3.0-only`）で提供します。
公開コンテナは、ビルドしたcommitのソースアーカイブを画面の「ライセンスとソース」から案内します。
GitHub Releaseにも同じcommitのソースとビルド手順を載せます。
コンテナ内の`/usr/share/doc/orca-server/`にライセンス本文と第三者通知を含めます。
同じ本文は実行バイナリに埋め込み、`/LICENSE`と`/THIRD_PARTY_NOTICES`で取得できます。

独自ビルドを配布・提供する場合は、その変更を含む対応ソースを取得できるURLを用意し、
ビルド時の`ORCA_SOURCE_URL`へ設定してください。実行時の環境変数では変更できません。
ソースにはRust/Svelteのコード、lockfile、Dockerfileと本書のビルド手順を含めます。

```sh
ORCA_SOURCE_URL=https://example.org/orca-server-source.tar.gz cargo build --locked --release
docker build --build-arg ORCA_SOURCE_URL=https://example.org/orca-server-source.tar.gz -t orca-server .
```

例のURLを、自分が配布するビルドに対応した公開先へ置き換えます。
未設定の開発ビルドでは公開先が未設定であることを画面へ表示し、公式版のソースへ誤って案内しません。
