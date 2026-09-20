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
| `PLATES_DIR` | `data/plates` | プレートの保存先。コンテナ内では`/data/plates`。書込み権限が必要です。 |
| `SCAD_LIVE_URL` | 未設定 | scad-liveのHTTP URL。未設定では取り込みAPIが503を返します。 |
| `ORCA_APPDIR` | 未設定 | 公式OrcaSlicer 2.4.2の展開先。詳細は[スライス](slicing.md)を参照。 |
| `ORCA_TIMEOUT_SECS` | `300` | 各CLI工程の上限秒数。OrcaSlicerを設定する場合は1〜3600。 |

ネイティブ実行では全IPv4インターフェースで待ち受けます。
到達範囲はホストのファイアウォールやコンテナのポート公開で制限します。
`GET /healthz`は`ok`、`GET /api/health`は`{"status":"ok"}`を返します。
プレートの操作と保存形式は[プレートAPI](plates.md)を参照してください。

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

一時保存先とscad-live互換のHTTP応答を用意し、STL選択・設定・保存・再検索・再取り込みを操作します。
RustサーバーとOrcaSlicerは実際に実行します。スクリーンショットと操作結果は指定した出力先へ保存します。
プリンターには接続しません。起動したサーバーと一時データは終了時に片付けます。

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
GitHub Releaseには公開イメージのdigestを記録します。

元のテンプレートは`fa63d25dbcb3762e2ecf7e56bfaddb994cdba07c`です。
MITの著作権表示はルートのLICENSEに保持しています。
