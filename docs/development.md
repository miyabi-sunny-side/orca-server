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
| `SCAD_LIVE_URL` | 未設定 | scad-liveのHTTP URL。モデル一覧・SCAD参照の保存/更新・試算と印刷時の取得に必要です。 |
| `ORCA_APPDIR` | ネイティブでは未設定、コンテナでは`/opt/orcaslicer` | 公式OrcaSlicer 2.4.2の展開先。詳細は[スライス](slicing.md)を参照。 |
| `DISCORD_WEBHOOK_URL` | 未設定 | Discordの完了通知。形式と配送条件は[通知設定](container.md#discordの完了通知)を参照。 |
| `ORCA_PUBLIC_URL` | 未設定 | 通知に付けるキューリンクの基準URL。省略するとリンクなし。 |
| `ORCA_TIMEOUT_SECS` | `300` | 各CLI工程の上限秒数。OrcaSlicerを設定する場合は1〜3600。 |

ネイティブ実行では全IPv4インターフェースで待ち受けます。
到達範囲はホストのファイアウォールやコンテナのポート公開で制限します。
`GET /healthz`は`ok`、`GET /api/health`は`{"status":"ok"}`を返します。
プレートの操作と保存形式は[プレートAPI](plates.md)を参照してください。

複数プリンターの台帳・環境変数からの初回取り込み・状態取得・印刷APIは[プリンター接続](printer.md)を参照してください。
[材料台帳・機種別設定・AMS対応API](filaments.md)も利用できます。
[MCP](mcp.md)は同じプロセスの`/mcp`で提供し、既存APIの検証・競合制御を使います。
未設定でもプレートの保存・閲覧は使えます。印刷予定の操作は[キューAPI](queue.md)を参照してください。

## 開発環境での検証

GitHub Actionsはformatter・型検査・lintと、画面・Rust・コンテナのビルドを行います。
テストは開発環境で実行します。Linuxで全項目を検証するには、次のコマンドを使います。

```sh
export ORCA_APPDIR=/path/to/squashfs-root
bash tests/verify-local.sh
```

Rust、Node.js、npmに加え、Docker Engine、OpenSSL、Python 3、curl、sha256sumが必要です。
[公式OrcaSlicer 2.4.2](slicing.md#orcaslicerの設定)も展開しておきます。
スクリプトはnpm依存とChromiumを取得します。Chromiumのシステム依存が足りなければ、次で導入します。

```sh
(cd client && npx playwright install --with-deps chromium)
```

依存の取得とコンテナのビルドにはネットワークを使います。必要なツールが使えない場合は失敗します。

全検証にはRustの単体・結合テスト、Vitest、Chromiumの模擬APIと実APIの操作、公式CLIを含みます。
リリースビルドの起動終了と、作成したコンテナでの配置・スライス・再起動も確認します。
プリンターや通知の接続先はループバック上の使い捨てサーバーです。実機やDiscordへの送信は行いません。
コンテナ検証ではLinuxのhostネットワークと専用の一時保存先を使います。

ログ・生成物・画面画像は表示された一時ディレクトリへ残します。
保存先は`ORCA_TEST_OUTPUT`で指定できます。プロセス・待受・一時DB・検証用コンテナは終了時に片付けます。
Python 3は運用コード`packaging/release_notes.py`の外部契約をRustから検証するために使います。
テスト本体、通信相手、CLI代替の実装はRustです。

### 変更に関係するテストを実行する

画面のビルド後、通常の隔離テストは`cargo test --locked`で実行できます。
MCPも自身で環境を起動するため、別の準備プロセスは不要です。

```sh
npm --prefix client run build
cargo test --locked
```

特定の結合テストだけを実行する場合は、先にCLI代替をビルドします。

```sh
cargo build --locked --example fixture-slicer
cargo test --locked --test queue_lifecycle
```

外部ツールを使うテストは通常実行でignoreと表示されます。以下は個別の実行例です。
全検証のスクリプトでは、これらを含む全対象を実行します。

```sh
cargo test --locked --test official_cli -- --ignored
ORCA_ROLE_UI=1 cargo test --locked --test material_roles -- --ignored --nocapture
cargo test --locked --test registry_flows independent_printers -- --ignored
cargo test --locked --test browser -- --ignored --test-threads=1 --nocapture
docker build -t orca-server-check .
ORCA_TEST_IMAGE=orca-server-check cargo test --locked --test container -- --ignored --test-threads=1
```

公式CLIとブラウザの全対象には`ORCA_APPDIR`が必要です。ブラウザの生成物保存先が共通なので、
ブラウザ対象は直列実行します。通知配送の通常テストは、内部のignore付きサービスを子プロセスとして起動します。
そのサービスだけを直接起動する必要はありません。

### 保証する範囲

Rustの結合テストはHTTP・MCP・SQLiteの保存と移行、同時操作、再起動、失敗後の保全を確認します。
MQTT/FTPSではv1/v3証明書、TLSセッション再利用、転送内容、AMS指定と重複命令を観測します。
通知は完了時の送信、重複防止、429・5xx・timeout・恒久エラー・中断を隔離したHTTPS相手で確認します。
Chromiumの操作と期待値はTypeScriptが所有し、実APIの起動・環境準備・通信制御をRustが担います。

`tests/common/slicer.rs`は状態遷移と設定受渡し用のCLI代替で、既知の印刷データを使います。
同梱の`tests/fixtures/p1_print.gcode.3mf`は、OrcaSlicer 2.4.2で20mm立方体2個を生成した通信検証用ファイルです。
P1S 0.4mm、0.20mm Standard、Generic PLA High Speed、Textured PEI Plateの構成です。
実際の配置・時間・押出経路は[公式CLI検証](slicing.md#cli連携の検証)で照合します。

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
docker build --target sources --output type=local,dest=/tmp/orca-sources .
```

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
