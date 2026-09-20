# OrcaServer

家庭内で使う3Dプリント用Webサービスの開発基盤です。
現在は接続確認と明暗テーマを備え、画面とAPIを1つの実行ファイルから配信します。
STLの取り込み・スライス・印刷には対応していません。

## 起動

Dockerを導入した環境で実行します。

```sh
docker run --rm -p 127.0.0.1:3000:3000 ghcr.io/miyabi-sunny-side/orca-server:latest
```

[ローカルの画面](http://127.0.0.1:3000)を開くと、接続結果が表示されます。
失敗した場合は「再試行」、テーマの変更は右上の「メニュー → テーマ設定」から操作します。
終了するには実行中のターミナルでCtrl+Cを押します。

この例はホスト内からのアクセスに限定しています。
他の端末へ公開する場合は、家庭内の信頼できるネットワークへ公開範囲を制限してください。

## 開発・設定

[開発ガイド](docs/development.md)に、ソースからの起動、環境変数、API、検証コマンドをまとめています。
画面の設計は[DESIGN.md](DESIGN.md)を参照してください。

## ライセンス

[MIT License](LICENSE)。[Rust + Svelte Template](https://github.com/miyabi-sunny-side/rust-svelte-template)を基にしています。
