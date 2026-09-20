# コンテナで使う

Linux x86_64向けイメージは、OrcaServerと公式OrcaSlicer 2.4.2、同じ版のprofileを含みます。
OrcaSlicerをホストへ導入する必要はありません。Node.js、デスクトップ、Xサーバー、GPUも不要です。

## 起動と接続

Dockerを用意し、[リリース一覧](https://github.com/miyabi-sunny-side/orca-server/releases)からイメージを選びます。
版を固定する場合は、リリースに記載された`image:tag@sha256:...`を使ってください。
次は最新公開版をホスト内へ公開する例です。

```sh
docker run --name orca-server --rm -p 127.0.0.1:3000:3000 \
  -v orca-plates:/data/plates \
  ghcr.io/miyabi-sunny-side/orca-server:latest
```

[ブラウザ](http://127.0.0.1:3000)を開きます。Ctrl+Cで停止してもボリュームは残ります。
プリンターが未設定・未接続でも起動し、保存・検索・スライスを利用できます。
印刷の開始には、[プリンターの登録と接続設定](printer.md#接続設定)と本体側の準備が必要です。

画面からSTLを選ぶには、コンテナから到達できるscad-liveのURLを`SCAD_LIVE_URL`へ指定します。
既存のDockerネットワーク上にあるサービスなら、そのサービス名を使えます。
ホストや別PC上のscad-liveには、そのPCのLANアドレスを使ってください。
コンテナ内の`localhost`はコンテナ自身です。

環境設定は、リポジトリ外のファイルを`--env-file /path/to/orca.env`で渡せます。
P1_*はDB初期化時だけ1台を取り込むための設定です。以後は画面から編集します。
次の例のアドレス・シリアル・コードを自分の環境の値へ置き換え、ファイルは所有者だけが読めるようにします。

```dotenv
SCAD_LIVE_URL=http://192.0.2.20:5003
P1_IP=192.0.2.10
P1_SERIAL=YOURPRINTERSERIAL
P1_ACCESS_CODE=YOUR_ACCESS_CODE
P1_TLS_CERT=/config/printer.pem
```

P1Sを設定する場合は、確認済みの証明書を読み取り専用でマウントします。
次のオプションを起動時に追加してください。

```sh
--env-file /path/to/orca.env -v /path/to/printer.pem:/config/printer.pem:ro
```
初回取り込みではP1の設定は一式を指定します。一部だけの指定や読めない証明書では起動しません。
初回から画面で登録する場合は、P1_*と証明書マウントは不要です。
スマホから開く場合はポートを家庭LANのアドレスへ公開し、信頼できるネットワークに到達範囲を制限します。

## 保存先と動作確認

実行ユーザーはUID/GID `10001:10001`です。名前付きボリュームは初回に保存先の所有権を引き継ぎます。
bind mountを使う場合は、ホスト上の保存先をこのUID/GIDで書き込めるように用意してください。
証明書にも同じユーザーの読取り権限を付けます。1つの保存先を複数プロセスで共有しないでください。

`/data/plates`は保存プレート、STL、設定、生成3MFと、プリンター台帳の`orca.sqlite3`を保持します。
台帳にはアクセスコードを含むため、保存先とバックアップを非公開にしてください。
更新前のrevisionも保存先に残ります。容量は全revisionのモデル・生成物と、一時コピーの余裕が必要です。
キューへ追加すると、その時点のSTL・設定・生成物をコンテナの`/tmp`へコピーします。
待機件数に応じた空き容量を確保してください。キューは再起動で消えます。
バックアップする場合は停止してから保存ボリューム全体を取得します。

HTTPは既定3000番です。`PORT`を変えた場合はDockerの公開先ポートも合わせます。
`GET /healthz`は`ok`、`GET /api/health`は`{"status":"ok"}`を返します。
DockerのhealthcheckはWebの応答を確認します。プリンターの接続可否は[状態API](printer.md#状態api)で確認します。

```sh
curl --fail http://127.0.0.1:3000/healthz
curl --fail http://127.0.0.1:3000/api/slicer/profiles
docker logs orca-server
```

## ライセンスとソース

OrcaServerとOrcaSlicerはAGPL v3です。各ライブラリにはそれぞれのライセンスが適用されます。
[第三者通知](../THIRD_PARTY_NOTICES)に出所・同梱物・通知の保存先を記載しています。
画面の「ライセンスとソース」とGitHub Releaseから、利用中のOrcaServerのソースを取得できます。

同じリリースの`OrcaSlicer-2.4.2-sources.tar.gz`は、OrcaSlicer本体、依存アーカイブ、wxWidgetsとsubmoduleを含みます。
本体の`build_linux.sh`、`.github/workflows/build_orca.yml`、`deps/`にビルド手順・設定・パッチがあります。
アーカイブの`wxWidgets-submodules.txt`と`slicer-sources.txt`で依存の版とハッシュを確認できます。
`OrcaServer-dependencies.tar.gz`にはCargo.lockに対応するRust依存ソースとvendor設定を含みます。
イメージ内にもRust依存とWeb側のMITコードの著作権・許諾表示を保持します。
OrcaSlicerの実行ファイル・launcher・profileは公式AppImageのままです。
AppImageの4つの補助共有ライブラリは、Ubuntuのexpat・lzma・mspack・zlibへ置き換えています。
Bambuの任意の非公開ネットワークプラグインは導入していません。

Ubuntuパッケージの版は`/usr/share/doc/orca-server/system-packages.txt`に記録します。
著作権・許諾は`/usr/share/doc/<package>/copyright`にあります。
対応ソースは[Ubuntuのソース配布](https://archive.ubuntu.com/ubuntu/pool/)から、パッケージと版を照合して取得できます。
改変したOrcaServerを配布する場合のソース指定と、イメージの再ビルドは[開発ガイド](development.md#ライセンスと対応ソース)を参照してください。
