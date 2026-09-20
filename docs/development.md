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

ネイティブ実行では全IPv4インターフェースで待ち受けます。
到達範囲はホストのファイアウォールやコンテナのポート公開で制限します。
`GET /healthz`は`ok`、`GET /api/health`は`{"status":"ok"}`を返します。

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

ブラウザテストは一時的なViteサーバーと隔離したAPI応答を使います。
RustのテストはHTTP応答と組み込み画面を確認します。
CIではビルドした実行ファイルを別ディレクトリから起動し、画面の配信も確認します。

## 構成

- `src/`: Axumのルーター、起動処理、環境変数の読込み。
- `client/`: Svelte 5の画面と検証コード。
- `build.rs`: ビルド済み画面の存在確認と変更追跡。
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
