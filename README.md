# OrcaServer

scad-liveのSTLを選び、印刷用プレートとして保存・検索する家庭内向けサーバーです。
ブラウザから複数モデルの選択、自動配置とスライス、配置確認、3MFの取得を行えます。
対象はBambu Lab P1Sの0.4mmノズル・単一材料です。プリンターへの送信には対応していません。
[P1Sの接続設定](docs/printer.md)を行うと、印刷状態とAMSトレイ情報をAPIから取得できます。

## 起動

Dockerを導入した環境で実行します。

```sh
docker run --rm -p 127.0.0.1:3000:3000 -v orca-plates:/data/plates ghcr.io/miyabi-sunny-side/orca-server:latest
```

[プレート一覧](http://127.0.0.1:3000)を開きます。保存データがなければ空の一覧を表示します。
この起動例では、[APIからのSTL保存・検索](docs/plates.md#保存と検索)と保存済みプレートの閲覧を利用できます。
公開コンテナにはOrcaSlicerを含まないため、自動配置を使う場合はLinuxで
[ソースから起動](docs/development.md#ソースから起動)し、[OrcaSlicerを設定](docs/slicing.md#orcaslicerの設定)してください。
画面から新規作成するには、[scad-liveの接続設定](docs/plates.md#scad-liveからの取り込み)も必要です。

プレートは`orca-plates`ボリュームへ保存され、コンテナ終了後も残ります。
終了するには実行中のターミナルでCtrl+Cを押します。
この例はホスト内からのアクセスに限定しています。
他の端末へ公開する場合は、家庭内の信頼できるネットワークへ公開範囲を制限してください。

## プレートを作る

scad-liveとOrcaSlicerを設定したサーバーで操作します。

1. 「新規作成」を開き、モデル名で検索してSTLを選びます。複数選択できます。
2. 「設定へ」でプレート名・工程・材料・プレート種類を確認し、「配置して保存」を押します。
3. 詳細画面で配置を確認し、「印刷データを取得」または「編集用3MFを取得」を選びます。
4. 保存済みプレートは、一覧の名前・モデル名の検索から開き直せます。

配置図はモデルの外形範囲を示す上面図です。精密な形状確認には3MFをOrcaSlicerで開いてください。
元のSTLが変わっても保存内容は変わりません。更新する場合は詳細の「元モデルを更新」から取り込み直します。
明暗テーマは端末設定に従います。右上のメニューから手動でも変更できます。

## 開発・設定

[開発ガイド](docs/development.md)に、環境変数、API、検証コマンドをまとめています。

## ライセンス

[MIT License](LICENSE)。[Rust + Svelte Template](https://github.com/miyabi-sunny-side/rust-svelte-template)を基にしています。
