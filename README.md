# OrcaServer

STLファイルをプレート単位で保存・検索する、家庭内向けのサーバーです。
scad-liveで作成したSTLの取り込みにも対応しています。
保存・検索はHTTP APIから操作します。画面は接続確認と明暗テーマに対応しています。
スライス・印刷には対応していません。

## 起動

Dockerを導入した環境で実行します。

```sh
docker run --rm -p 127.0.0.1:3000:3000 -v orca-plates:/data/plates ghcr.io/miyabi-sunny-side/orca-server:latest
```

[プレートAPI](docs/plates.md)の手順で、手元のSTLやscad-liveのモデルを保存できます。
[接続確認画面](http://127.0.0.1:3000)に接続成功が表示されれば、サーバーは起動しています。
プレートは`orca-plates`ボリュームへ保存され、コンテナ終了後も残ります。
終了するには実行中のターミナルでCtrl+Cを押します。

この例はホスト内からのアクセスに限定しています。
他の端末へ公開する場合は、家庭内の信頼できるネットワークへ公開範囲を制限してください。

## 開発・設定

[開発ガイド](docs/development.md)に、ソースからの起動、環境変数、API、検証コマンドをまとめています。

## ライセンス

[MIT License](LICENSE)。[Rust + Svelte Template](https://github.com/miyabi-sunny-side/rust-svelte-template)を基にしています。
