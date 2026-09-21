# プリンターの登録・接続・印刷

P1SへMQTT/TLSで接続し、印刷状態・進捗・エラー番号・AMSトレイ情報を取得します。
印刷準備で生成した3MFをFTPSで転送し、指定したAMSトレイから印刷を開始できます。
同梱OrcaSlicerのBBLプロファイルから、単一ノズル構成を選べます。
通信は既存のBambu LAN経路、印刷は単一材料・AMSからの供給です。P1S以外の通信・実印刷は未確認です。
クラウド認証や自動発見は行いません。
印刷の開始と復旧には[キューAPI](queue.md)を使います。

## 画面から登録する

メニューの「プリンター」で一覧を開き、「追加」を選びます。
名前、機種・装着ノズル径、材質、既定の工程、プレート種類を入力します。
接続情報はIPアドレス、シリアル、LANアクセスコード、TLS証明書のPEM本文です。
保存後は接続状態を機器ごとに表示します。未接続でも編集・削除できます。

機種と径はOrcaのmachine profileで識別します。工程・材料の候補はその構成に対応するものだけです。
ノズル材質はステンレス・焼入れ鋼・未確認から選びます。登録は物理センサーによる装着確認ではありません。
交換後は本体の設定も合わせてください。本体が申告する径・材質を受信できた場合は、印刷前にも照合します。

機器名または「設定を編集」から変更します。アクセスコード・証明書は表示せず、空欄なら既存値を維持します。
準備中・印刷中・取り外し待ち・要確認でも、名前・既定の工程・プレート種類は変更できます。
工程とプレート種類は次回の初期入力用で、機器へ再接続せず、進行中の印刷と保存済み条件を維持します。
待機ジョブだけなら接続先・ノズルを変更できますが、ジョブの要求機種は維持します。不一致は保留となります。
機器を削除するには、先に全ジョブを終了・取消してください。

一覧では、[新規プレートの初期値](plates.md#新規作成の初期値)に使う機器も選べます。1台だけなら自動で選択します。

## SQLiteと初回取り込み

`<PLATES_DIR>/orca.sqlite3`の`printers`テーブルへ設定を保存します。schemaは
SQLiteの`PRAGMA user_version`で管理します。初期化と環境変数からの取り込みは同じtransactionで確定します。
失敗すれば未完了の初期化を取り消します。新しいschemaを古い実行ファイルで開く場合は、書き換えずに起動を拒否します。

DB初期化時だけ、下記の`P1_*`を1台分として取り込みます。この機器のIDは`p1`です。
設定がなければ空の台帳を作ります。その後の再起動では環境変数を読み直さず、UI/APIの変更や削除を維持します。
空の台帳を初期化した後に`P1_*`を追加しても取り込みません。その場合は画面から登録してください。

DBにはアクセスコードと証明書の本文も保存するため、作成時のファイル権限は所有者のみ読書きできる0600です。
バックアップも非公開で保管します。停止したサーバーの保存ボリューム全体を取得してください。
旧版へ戻す場合は、その版の保存領域とイメージを組にして復元します。新しいDBのversionだけを書き換えないでください。
プレート構成とキューも同じDBへ保存します。[更新とバックアップ](container.md#更新とバックアップ)に旧ファイル形式の移行条件をまとめています。

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

プレートをキューへ追加し、予定材料・AMSスロット・要求機種とノズルを指定します。
造形物を除去して空のビルドプレートを戻してから、[キューの開始操作](queue.md#次の印刷と再送)を行います。
開始前と転送後に機器・AMS・材料設定を照合します。残量や物理的な装着状態は利用者も確認してください。

キューの`preparing`は、モデル取得から機器の開始確認までを含みます。
`GET /api/printer/status?printer_id=PRINTER_ID`の`start`で通信段階を確認できます。
単一材料のスライサーID 1を、選んだ従来AMSのunit 0〜3・tray 0〜3へ対応づけます。
外部スプールや装填未確認のスロットでは開始しません。

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

機器ごとの進行中ジョブはDB制約で1件に限ります。別プリンターのジョブは独立しています。
試行ID・固定入力・開始前の記録を永続化してから送信します。
転送は最大5分で終了し、データ接続では制御接続のTLSセッションを再利用します。
MQTT命令はretainなしのQoS 0で1回だけ要求し、通信断・タイムアウトで自動再送しません。
確認待ちがタイムアウトした場合は接続を切り、送信待ちのデータを破棄して状態を再取得します。
アップロード済みファイルは、要求ごとに異なる`orca-<UUID>.gcode.3mf`名でSDカードへ残ります。
不要になったものはプリンター側のファイル管理で削除できます。

`unknown`の場合は本体を確認します。印刷中なら新しい要求を出さず、報告の復帰を待ってください。
再起動・通信断・DB復元後も開始命令を自動再送しません。不明状態の解除や再準備は、キューの確認操作で行います。
完了だけで次の印刷を自動実行しません。

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
| `start` | 最後の開始要求。未要求は`null`。`id`、`plate_id`、`job_id`、`ams_slot`、`phase`、`message`。 |

`print.state`は機器の文字列です。未知の値、`PAUSE`、`FAILED`、状態不明を待機中と扱いません。
`ready_to_print`は機器側の状態条件を表し、造形物を除去したかどうかは判定しません。

AMSの各unitは`id`、`humidity`、`trays`を持ちます。湿度は機器が返す段階値で、百分率ではありません。
trayは`id`、`present`、`material`、`color`、`remaining_percent`のほか、材料ID・銘柄・タグ・温度範囲・最終観測時刻を持ちます。
台帳との対応と永続化は[材料管理ガイド](filaments.md)を参照してください。
在席状態が不明なら`present`は`null`、空のトレイでは材料情報を解除します。色はRRGGBBAAです。
AMS IDは0〜255を保持し、各trayは0〜3です。在席ビットで確認できるのは従来AMSのunit 0〜3です。
未確認のIDは印刷用の選択肢へ加えません。
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
