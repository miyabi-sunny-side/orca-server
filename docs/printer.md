# P1Sの状態取得

P1SへMQTT/TLSで接続し、印刷状態・進捗・エラー番号・AMSトレイ情報を取得します。
この接続機能は状態を読むためのものです。印刷開始やクラウド認証、自動発見は行いません。

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

全て未設定なら接続機能を無効にします。一部だけの指定や不正値、読めない証明書ではサーバーが起動しません。
設定は環境変数から読み、プレートの保存先へコピーしません。アクセスコードをAPI応答やログには含めません。

### プリンター証明書

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
| `print` | `state`、`percent`、`remaining_minutes`、`error`、`job_id`、`file`。不明な項目は`null`。 |
| `ams` | `current_tray`と`units`。AMS情報が未取得なら`null`。 |

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
MQTTパケットやライブラリのエラー詳細は、秘密値の混入を避けるためログへ出しません。

実装は[Bambu Studioの状態処理](https://github.com/bambulab/BambuStudio/blob/77b9dd94d1e3c432d5e74a18ab8de146ccf3b7c7/src/slic3r/GUI/DeviceManager.cpp)と、
[ha-bambulabのLAN接続](https://github.com/greghesp/ha-bambulab/blob/cd67ed90e08561175a831f35b45773fe427996d7/custom_components/bambu_lab/pybambu/bambu_client.py)、
[OpenBambuAPIのP1 report](https://github.com/Doridian/OpenBambuAPI/blob/cc383a2c96576a9f53391c879acfb8dca5c534e4/mqtt.md)を参照しています。
隔離検証の手順は[開発ガイド](development.md#検証)を参照してください。実機のファームウェアによる互換性は接続先で確認してください。
