# プリンターの登録・接続・印刷

P1SへMQTT/TLSで接続し、印刷状態・進捗・エラー番号・AMSトレイ情報を取得します。
保存済みの印刷3MFをFTPSで転送し、指定したAMSトレイから印刷を開始できます。
同梱OrcaSlicerのBBLプロファイルから、単一ノズル構成を選べます。
通信は既存のBambu LAN経路、印刷は単一材料・AMSからの供給です。P1S以外の通信・実印刷は未確認です。
クラウド認証や自動発見は行いません。
複数のプレートを順に扱う場合は[キューAPI](queue.md)を使います。

## 画面から登録する

メニューの「プリンター」で一覧を開き、「追加」を選びます。
名前、機種・装着ノズル径、材質、既定の工程、プレート種類を入力します。
接続情報はIPアドレス、シリアル、LANアクセスコード、TLS証明書のPEM本文です。
保存後は接続状態を機器ごとに表示します。未接続でも編集・削除できます。

機種と径はOrcaのmachine profileで識別します。工程・材料の候補はその構成に対応するものだけです。
ノズル材質はステンレス・焼入れ鋼・未確認から選びます。登録は物理センサーによる装着確認ではありません。
交換後は本体の設定も合わせてください。本体が申告する径・材質を受信できた場合は、印刷前にも照合します。

機器名または「設定を編集」から変更します。アクセスコード・証明書は表示せず、空欄なら既存値を維持します。
印刷中・開始結果が不明・キューにジョブがある機器は、名前だけ変更できます。
接続先・ノズルなどの変更や機器の削除は、先にジョブを終えてから行います。待機中のジョブはキューから除くこともできます。

## SQLiteと初回取り込み

`<PLATES_DIR>/orca.sqlite3`の`printers`テーブルへ設定を保存します。現在のschema versionは1で、
SQLiteの`PRAGMA user_version`で管理します。初期化と環境変数からの取り込みは同じtransactionで確定します。
失敗すれば未完了の初期化を取り消します。新しいschemaを古い実行ファイルで開く場合は、書き換えずに起動を拒否します。

DB初期化時だけ、下記の`P1_*`を1台分として取り込みます。この機器のIDは`p1`です。
設定がなければ空の台帳を作ります。その後の再起動では環境変数を読み直さず、UI/APIの変更や削除を維持します。
空の台帳を初期化した後に`P1_*`を追加しても取り込みません。その場合は画面から登録してください。

DBにはアクセスコードと証明書の本文も保存するため、作成時のファイル権限は所有者のみ読書きできる0600です。
バックアップも非公開で保管します。停止したサーバーの保存ボリューム全体を取得してください。
旧版へ戻す場合は、その版の保存領域とイメージを組にして復元します。新しいDBのversionだけを書き換えないでください。
プレートの既存ファイル・ID・revisionはこの移行で変更しません。キューは引き続きプロセス内に保持します。

### 台帳API

| メソッドとパス | 操作 |
| --- | --- |
| `GET /api/printers` | 設定・接続状態の一覧。 |
| `POST /api/printers` | 設定をJSONで追加。201と生成したIDを返します。 |
| `GET /api/printers/{id}` | 指定機器の設定・接続状態。 |
| `PUT /api/printers/{id}` | 設定をJSONで更新。IDは変わりません。 |
| `DELETE /api/printers/{id}` | 未使用の機器を削除。成功は204。 |
| `GET /api/printers/profiles` | 使用可能なmachine profileのkey・機種・ノズル径。 |

POST/PUTには名前と接続先として`name`、`host`（IPアドレス）、`serial`が必要です。
構成は`machine_profile_key`、`default_process_profile_key`、`bed_type`、`nozzle_material`で指定します。
材質は`stainless_steel`、`hardened_steel`、`unknown`です。
POSTでは`access_code`と`tls_certificate`（PEM本文）も必要です。PUTでは省略・空文字で保持します。
`mqtt_port`、`ftps_port`、`start_timeout_secs`の既定値は8883、990、600です。

応答は秘密値を含めず、`status`に接続・印刷状態、`machine`にプロファイル由来の機種・径を返します。
保存したプロファイルが消失した場合は`configuration_error`を表示し、その機器の操作を拒否します。
機器の一覧・修正・削除は利用できます。未知のprofileをP1Sへ置き換えることはありません。
Orcaを導入しない監視専用の旧構成では、既存P1S 0.4mm・標準工程だけを利用できます。

## 接続設定

サーバーから到達できるP1SのIPアドレス、シリアル番号、LANアクセスコードを用意します。
端末側のLAN接続を許可する設定は、利用中のファームウェアの案内に従ってください。

| 環境変数 | 内容 |
| --- | --- |
| `P1_IP` | プリンターのIPv4またはIPv6アドレス。 |
| `P1_SERIAL` | シリアル番号。MQTT topicの識別に使用します。 |
| `P1_ACCESS_CODE` | LANアクセスコード。サーバー側だけで使用します。 |
| `P1_TLS_CERT` | 接続を許可するプリンター証明書1枚を持つPEMファイル。 |
| `P1_MQTT_PORT` | 既定8883。変更する場合は1〜65535の整数。 |
| `P1_FTPS_PORT` | 既定990。implicit FTPSの制御ポート。データ接続はPASVで通知されたポートを使います。 |
| `P1_START_TIMEOUT_SECS` | 既定600。送信後に印刷開始を確認するまでの上限秒数。1〜3600。 |

上記の環境変数はDB初期化時の取り込み専用です。全て未設定なら空の台帳を作ります。
初期化時に一部だけの指定や不正値、読めない証明書があれば起動しません。既存DBの設定を環境変数で上書きしません。

### プリンター証明書

MQTTとFTPSで同じプリンター証明書を使用する構成に対応します。
指定した証明書と接続先の証明書は、DERのバイト列で照合します。一致しなければ接続しません。
自己署名でも利用できます。
この設定はP1接続専用です。証明書の自動信頼や、アプリ全体のTLS検証の無効化は行いません。
TLS 1.2を使用し、証明書の公開鍵でハンドシェイクの署名を検証します。
CA・ホスト名・有効期限による判定は使いません。証明書が変わった場合は、管理者が接続先を確認してPEMを更新します。

管理するプリンターのIPを`P1_IP`へ指定し、証明書を取得する例です。
取得先が自分のP1Sであることを確認して配置してください。

```sh
openssl s_client -connect "${P1_IP}:8883" -showcerts </dev/null 2>/dev/null | openssl x509 -out printer.pem
openssl x509 -in printer.pem -noout -fingerprint -sha256
```

IPv6の`-connect`引数は`[アドレス]:8883`とします。
秘密鍵は不要です。PEMはサーバーの実行ユーザーから読み取れる場所に置きます。

### サーバーへ渡す

次の値を利用環境に置き換えます。設定ファイルを使う場合はリポジトリ外へ置き、アクセス権を所有者だけに制限します。

```sh
export P1_IP=192.0.2.10
export P1_SERIAL=YOURPRINTERSERIAL
export P1_ACCESS_CODE=YOUR_ACCESS_CODE
export P1_TLS_CERT="$PWD/printer.pem"
cargo run --locked
```

事前の画面ビルドは[開発ガイド](development.md#ソースから起動)を参照してください。
Dockerでは環境変数を`--env-file`で渡し、証明書を読み取り専用でマウントします。
`P1_TLS_CERT`にはコンテナ内のパスを指定してください。実行ユーザーのUID/GIDは10001です。

## 印刷を開始する

[スライスAPI](slicing.md)で印刷3MFを保存したプレートを使います。
造形物を除去してビルドプレートを戻し、使用するAMSの材料とノズルを確認してください。
`PLATE_ID`と`REVISION`は`GET /api/plates`の`id`と`revision`に置き換えます。
`PRINTER_ID`は`GET /api/printers`のIDです。状態・開始・不明状態の解除・キューには同じ`printer_id`を付けます。
1台だけなら省略できますが、複数登録時の省略は409です。不明なIDでは別の機器へ切り替えません。
以下は選択した機器の最初のAMSの4番目のトレイを選ぶ例です。

```sh
curl --fail-with-body -X POST "http://127.0.0.1:3000/api/plates/PLATE_ID/print?printer_id=PRINTER_ID" \
  -H 'Content-Type: application/json' \
  -d '{"revision":"REVISION","ams_slot":3}'
```

選択した機器のmachine profileと、3MFの対象機器が一致しない場合は拒否します。
202は要求の受付を示し、印刷開始を保証しません。`GET /api/printer/status`の`start`で経過を確認します。
`ams_slot`は0〜15の整数で、`unit × 4 + tray`です。単一材料のスライサーID 1を、
MQTTの`ams_mapping`の先頭から選択トレイへ対応づけます。
トレイが在席・材料既知で、3MFの材料種別と一致する場合に開始できます。
色や残量の一致は確認しないため、使用前に確認してください。

| `start.phase` | 意味 |
| --- | --- |
| `uploading` | FTPS転送中。まだ開始命令を送っていません。 |
| `awaiting_confirmation` | 転送成功後にMQTTへ開始命令を渡し、プリンターの報告を待っています。 |
| `accepted` | 要求番号の一致する受理応答、または同じ印刷名の準備状態を確認しました。 |
| `printing` | 同じ印刷名またはファイル名の`RUNNING`を確認しました。 |
| `finished` | 印刷開始を確認した要求について`FINISH`を受信しました。 |
| `upload_failed` / `not_sent` | 転送失敗、接続・AMS状態の変化などで開始命令を送っていません。 |
| `rejected` | プリンターが要求を拒否しました。 |
| `unknown` | 通信断・タイムアウト・エラー・一時停止、または印刷中から完了報告なしで待機へ戻った状態。 |
| `resolved` | 利用者が端末を確認し、不明状態を解除しました。 |

機器ごとの開始は同時に1件だけです。状態不明・印刷中・エラー・未解決の開始要求がある場合は409を返します。
保存後にrevisionが変わった場合も409です。未設定は503、不正な3MFは400です。
転送は最大5分で終了し、データ接続では制御接続のTLSセッションを再利用します。
MQTT命令はretainなしのQoS 0で1回だけ要求し、通信断・タイムアウトで自動再送しません。
確認待ちがタイムアウトした場合は接続を切り、送信待ちのデータを破棄して状態を再取得します。
アップロード済みファイルは、要求ごとに異なる`orca-<UUID>.gcode.3mf`名でSDカードへ残ります。
不要になったものはプリンター側のファイル管理で削除できます。

`unknown`の場合は端末を確認してください。印刷中なら新しい要求を出さず、報告の復帰を待ちます。
結果を確認し、プリンターが同期済みの待機状態なら、次の操作で開始要求だけを解除できます。
`START_ID`は`start.id`に置き換えます。この操作で印刷を停止したり、ファイルを削除したり、命令を再送したりすることはありません。

```sh
curl --fail-with-body -X POST "http://127.0.0.1:3000/api/printer/start/START_ID/resolve?printer_id=PRINTER_ID" \
  -H 'Content-Type: application/json' -d '{"checked_printer":true}'
```

開始要求はメモリに1件保持し、サーバー再起動で消えます。再起動後も全状態を確認するまで開始しません。
`finished`だけで次の印刷を自動実行しません。利用者が造形物を除去してから次を要求します。

## 状態API

```sh
curl --fail http://127.0.0.1:3000/api/printer/status
```

未設定でも200とJSONを返し、`connection`は`unconfigured`、`ready_to_print`は`false`になります。
接続しただけでは同期完了にしません。report topicの購読成功後、request topicへ`pushall`を送信します。
全状態の`push_status`を受信してから差分更新を適用します。retained reportや別端末のtopicは無視します。

| 項目 | 内容 |
| --- | --- |
| `connection` | `unconfigured`、`connecting`、`synchronizing`、`connected`、`disconnected`、`stale`。 |
| `synchronized` | 接続中で全状態を確認し、情報が新しい場合だけ`true`。 |
| `updated_at` | 最後に採用した状態の受信時刻。Unix秒。未取得は`null`。 |
| `ready_to_print` | 同期済み・60秒未満・`IDLE`または`FINISH`・エラー0の場合だけ`true`。 |
| `print` | `state`、`percent`、`remaining_minutes`、`error`、`job_id`、`file`、`name`。不明な項目は`null`。 |
| `ams` | `current_tray`と`units`。AMS情報が未取得なら`null`。 |
| `start` | 最後の開始要求。未要求は`null`。`id`、`plate_id`、`revision`、`ams_slot`、`phase`、`message`。 |

`print.state`は機器の文字列です。未知の値、`PAUSE`、`FAILED`、状態不明を待機中と扱いません。
`ready_to_print`は機器側の状態条件を表し、造形物を除去したかどうかは判定しません。

AMSの各unitは`id`、`humidity`、`trays`を持ちます。湿度は機器が返す段階値で、百分率ではありません。
trayは`id`、`present`、`material`、`color`、`remaining_percent`を持ちます。
在席状態が不明なら`present`は`null`、空のトレイでは材料情報を解除します。色はRRGGBBAAです。
対象は従来AMSのunit 0〜3、各tray 0〜3です。未報告のIDを推測で選択可能にしません。
`current_tray`は`unit × 4 + tray`、254は外部スプール、255は選択なしです。

部分更新では未報告の値を保持し、トレイはIDごとに更新します。全状態の受信では以前の値をリセットします。
通信断では印刷可能の判定を解除し、再接続後も新しい全状態を要求します。以前の要求は再送しません。
再接続は5秒おきです。接続中に未同期・情報不足となった場合は、5分おきに全状態を再要求します。
報告が60秒以上途切れた状態は`stale`です。その後に差分だけを受けても同期完了へ戻しません。

## 接続できない場合

`connection`とサーバーログを確認し、IP・シリアル・アクセスコード・証明書・到達できるポートを照合してください。
MQTTパケットやFTPSの応答、ライブラリのエラー詳細は、秘密値の混入を避けるためログへ出しません。

実装は[Bambu Studioの状態処理](https://github.com/bambulab/BambuStudio/blob/77b9dd94d1e3c432d5e74a18ab8de146ccf3b7c7/src/slic3r/GUI/DeviceManager.cpp)と、
[ha-bambulabのLAN接続](https://github.com/greghesp/ha-bambulab/blob/cd67ed90e08561175a831f35b45773fe427996d7/custom_components/bambu_lab/pybambu/bambu_client.py)、
[OpenBambuAPIのP1 report](https://github.com/Doridian/OpenBambuAPI/blob/cc383a2c96576a9f53391c879acfb8dca5c534e4/mqtt.md)を参照しています。
隔離検証の手順は[開発ガイド](development.md#検証)を参照してください。実機のファームウェアによる互換性は接続先で確認してください。

FTPSは[SuppaFTP 12.0.1](https://docs.rs/suppaftp/12.0.1/suppaftp/)のTokio/Rustls実装を使用します（MITまたはApache-2.0）。
印刷命令は[ha-bambulabのproject_file](https://github.com/greghesp/ha-bambulab/blob/cd67ed90e08561175a831f35b45773fe427996d7/custom_components/bambu_lab/pybambu/commands.py)を参照しています。非公開のBambu network pluginは含めません。
