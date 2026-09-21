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
プリンターが未設定・未接続でも起動し、保存・検索を利用できます。
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

## Discordの完了通知

`DISCORD_WEBHOOK_URL`でDiscordのIncoming Webhookを指定します。
管理中の印刷が完了して「取り外し待ち」になると通知します。
プリンター名・プレート名・ジョブIDを送ります。送信受理、進捗100%、失敗・取消、取り外しだけでは通知しません。
未設定・空文字なら通知は無効です。

次の2形式を受け付けます。`<id>`と`<token>`は自分のWebhookの値に置き換えます。

- `https://discord.com/api/webhooks/<id>/<token>`
- `discord://<token>@<id>`（Watchtower/Shoutrrrで使う形式）

queryやfragmentを付けないでください。不正な形式では起動せず、ログに値は出しません。
URLには秘密のtokenが含まれるため、上記の非公開envファイルへ保存します。
すでにCompose用の`WATCHTOWER_NOTIFICATION_URL`で送り先を管理している場合は、サービスの設定で参照できます。

```yaml
environment:
  DISCORD_WEBHOOK_URL: ${WATCHTOWER_NOTIFICATION_URL}
```

キューへのリンクも通知する場合は、ブラウザから開けるサーバーのURLを`ORCA_PUBLIC_URL`へ指定します（例: `https://orca.example/`）。
HTTP/HTTPSのURLで、ユーザー名・パスワード・query・fragmentは指定できません。末尾の`/`を補ったURLは768 byte以下にします。
未設定ならリンクを省略します。プリンター名・プレート名によるメンションは無効にします。

送信は印刷処理と別に行い、通信障害でキューや取り外し操作を止めません。
完了時に通知を保存するため、取り外しでジョブを削除しても送信対象は残ります。
Discordの応答でメッセージIDを確認できたものは、通常の再起動やFINISHの繰返しで再送しません。
導入前や通知無効中に完了したジョブを、後からまとめて通知することはありません。

HTTP送信は1回10秒まで、1件あたり最大3回です。429は指定された待ち時間を守り、5xxと通信失敗は2秒・4秒の間隔で再試行します。
認証エラーなどの恒久的な4xxでは再試行しません。送信結果をDBへ保存できない間は、HTTPを再送せず保存を再試行します。
応答消失やプロセス停止で到達不明になると、再試行で通知が重複し得ます。
1回限りの配送は保証しません。3回とも結果不明なら、自動再送を止めます。

`docker logs orca-server`で`Discord completion notification delivery recorded`を確認できます。
`sent`は到達確認済み、`pending`は再試行待ち、`unknown`は到達不明、`failed`は打切りです。`tries`は送信回数です。
初期状態や送信中も含む記録はSQLiteの`print_notifications`に残ります。送信先URLやtokenは保存しません。
設定後は通常の印刷を1件完了させ、指定チャンネルの通知とキューの「取り外し待ち」を照合してください。

## 保存先と動作確認

実行ユーザーはUID/GID `10001:10001`です。名前付きボリュームは初回に保存先の所有権を引き継ぎます。
bind mountを使う場合は、ホスト上の保存先をこのUID/GIDで書き込めるように用意してください。
証明書にも同じユーザーの読取り権限を付けます。1つの保存先を複数プロセスで共有しないでください。

`/data/plates/orca.sqlite3`にプレート構成・アップロード元STL・プリンター・材料・AMS・印刷キューを保存します。
DBにはアクセスコードも含むため、保存先とバックアップを非公開にしてください。
準備中以降の固定入力と生成物は`/data/plates/jobs/`へ保存し、完了・取消後に片付けます。
待機ジョブはSTLの複製を持ちません。準備時のSTL・生成物に必要な空き容量を確保してください。

HTTPは既定3000番です。`PORT`を変えた場合はDockerの公開先ポートも合わせます。
`GET /healthz`は`ok`、`GET /api/health`は`{"status":"ok"}`を返します。
DockerのhealthcheckはWebの応答を確認します。プリンターの接続可否は[状態API](printer.md#状態api)で確認します。

```sh
curl --fail http://127.0.0.1:3000/healthz
curl --fail http://127.0.0.1:3000/api/slicer/profiles
docker logs orca-server
```

## 更新とバックアップ

更新前に印刷と取り外しを終え、サーバーを停止して保存ボリューム全体と使用中イメージの版を保管します。
稼働中のDBファイルだけをコピーしないでください。SCAD元データはscad-live側でも保管します。
アップロード元STLは同じDBに含まれるため、DBの復元で再利用できます。

現在のSQLite schema versionは8です。機器・材料には`printers`、`filament_products`、`filaments`、`filament_settings`、`ams_slots`を使います。
プレート・キューには`plates`、`plate_items`、`print_jobs`を使います。
初期設定は`default_settings`、通知は`print_notifications`で保持します。
製品と色の分離では既存材料IDとキューを保持し、全設定が一致する製品だけをまとめます。[材料の移行条件](filaments.md#保存と移行)を確認してください。
旧版からの初回起動では、旧`plate.json`が示すプレートID・名前・モデル構成を一つのtransactionで取り込みます。
SCAD由来は参照、直接アップロード由来はSTL本体を保管し、個数は1です。
旧revision・スライス設定・生成物の履歴は新しい再利用プレートへ引き継ぎません。
プレートに新設した4つの印刷条件はNULLのまま移行し、進行中の試行・固定入力は保持します。
移行後は[プレートの条件](plates.md#印刷条件を保存する)と材料設定を確認してから追加・印刷してください。

公開済みの旧プレートが壊れていれば移行全体を取り消し、起動を拒否します。旧ファイルを自動削除・上書きしません。
バックアップとログを確認して旧データを修復してから再起動します。DBを消して取り込みを強制しないでください。
成功後はDBを使用し、旧ファイルを再取り込みしません。旧ファイルは保管が不要と判断するまで残してください。

キューは再起動・復元後も保持し、不明な開始は要確認になります。待機報告だけで未印刷と判断せず、命令も再送しません。
[復旧操作](queue.md#失敗・再起動からの復旧)に従って本体を確認してください。
旧版へ戻す場合は、更新前の保存領域と対応するイメージを組にして復元します。
DBの`user_version`だけを書き換えて古い実行ファイルで開くことはできません。

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
