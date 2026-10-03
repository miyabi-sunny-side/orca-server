# P1S・A1 miniの機能対応表

P1S本体とBambu Studio/HandyがLAN（MQTT・FTPS・カメラ）で行う通常の操作を、OrcaServerから使えるかで分類します。
「実装」はAPI・画面・MCPの入口と隔離環境での検証があるもの、「LAN不可」「本体で操作」は根拠を示したものです。
実機での確認は受入時に行います。sandboxの模擬だけで実機の動作を保証しません。

根拠の一次資料は[BambuStudio 77b9dd9](https://github.com/bambulab/BambuStudio/tree/77b9dd94d1e3c432d5e74a18ab8de146ccf3b7c7/src/slic3r/GUI)の命令・画面のソースです。
主に`DeviceManager.cpp`、`DeviceCore/*`、`SelectMachine.cpp`、`StatusPanel.cpp`を参照しました。
LAN印刷とカメラは[ha-bambulab cd67ed9](https://github.com/greghesp/ha-bambulab/tree/cd67ed90e08561175a831f35b45773fe427996d7/custom_components/bambu_lab)も参照しました。
本番P1Sの値は2026-10-03の[通信記録](printer.md#通信記録api)（`flag3=15`、`fun`なし、`home_flag=24331536`）です。

## 給材

| 機能 | 状態 | 入口・根拠 |
| --- | --- | --- |
| AMSから印刷 | 実装 | キューの給材`ams`。指定材料のslotを使用順で解決。 |
| 外部スプールから印刷 | 実装 | キューの給材`external`。`use_ams:false`で開始（SelectMachine.cpp、ha-bambulab）。 |
| AMSを外した後の継続 | 実装 | AMSの消失で既存ジョブを切り替えず、行メニューで給材を変更。 |
| ロード・アンロード | 実装 | 操作`load`（slot 0〜15、外部254）・`unload`。`ams_change_filament`。 |
| AMSのエラー後の再試行・完了 | 実装 | 操作`ams`（`resume`・`reset`・`done`）。`ams_control`。 |
| 材料切れ時の標準自動補充 | 実装 | AMS画面の自動補充（`print_option.auto_switch_filament`）。 |
| 起動時・挿入時の読取、残量推定 | 実装 | 操作`ams_reading`。`ams_user_setting`。 |
| トレイの再読取 | 実装 | 操作`read_tray`。`M620 R`（command_ams_refresh_rfid）。 |
| 本体へトレイの材料を伝える | 実装 | 操作`tray_setting`。`ams_filament_setting`。外部スプールはams 255・tray 254。 |
| 外部1本での多色（手動交換補助） | LAN不可 | 本体が対応を報告しない（`flag3` bit 16・`fun` bit 48が0）。異なる材料のプレートは理由を示して保留。 |
| AMSの乾燥 | 本体にない | P1S用AMSに乾燥機能がない（`auto_stop_ams_dry`はAMS 2 Pro/HT用）。 |

## ライフサイクル

| 機能 | 状態 | 入口・根拠 |
| --- | --- | --- |
| 一時停止・再開・停止 | 実装 | ホームの現在ジョブと本体操作ページ、操作`pause`・`resume`・`stop`。一時停止は失敗扱いにしない。 |
| 取消・再印刷・除外 | 実装 | キューの`remove`・`retry`・`discard`。 |
| 完了と次の印刷 | 実装 | キューの`next`。空のプレートの確認を伴う明示操作だけで開始。 |
| 本体側で開始・停止した印刷 | 実装 | 状態APIの`print`と実況。対象外の印刷中はキューを開始しない。 |
| サーバー・本体の再起動、電源断 | 実装 | 不明な開始を再送せず、再接続後の空の待機報告で解除。 |
| 名前表記の違う本体の報告（`orca-<id>.gcode.3mf`） | 実装 | 試行の識別でファイル名を印刷名として受け付ける（2026-10-01の本番事例）。 |
| オブジェクトのスキップ | 実装 | 現在ジョブの`objects`と操作`skip_objects`。 |
| SDカードのファイルから印刷 | 本体で操作 | 開始はキューを通し、試行の追跡と空プレートの確認を保つ。SD上のファイルの印刷は本体画面のファイル一覧で行う。 |

## 条件

| 機能 | 状態 | 入口・根拠 |
| --- | --- | --- |
| 開始オプション | 実装 | プレートの`start_options`（ベッドレベリング・フロー較正・タイムラプス・振動補正）。既定はStudioの印刷ダイアログ。 |
| 印刷速度 | 実装 | 操作`speed`（1静音〜4ルーディクラス）。`print_speed`。 |
| ノズル・ベッド温度 | 実装 | 操作`nozzle_temperature`・`bed_temperature`。`M104`/`M140`（P1Sは`set_bed_temp`非対応）。 |
| ファン | 実装 | 操作`fan`（部品冷却・補助・チャンバー）。`M106 P1/P2/P3`。 |
| 照明 | 実装 | 操作`light`。`ledctrl`。 |
| 軸移動・ホーム・押出 | 実装 | 操作`move`・`home`・`extrude`。DevAxisCtrl.cppのG-code。A1系はY・Zの向きを反転。 |
| 本体キャリブレーション | 実装 | 操作`calibrate`（ベッドレベリング・振動・モーターノイズ）。`calibration`。 |
| 自動復旧・通知音 | 実装 | 操作`auto_recovery`・`sound`。本体が対応を報告しない項目（P1Sの通知音）は画面に出さない。 |
| 本体での自動フロー較正（PA） | LAN不可 | BambuStudioはPシリーズで`is_support_pa_calibration`を無効化。手動較正は較正モデルの通常印刷で行う。 |
| AI検知（スパゲッティ・初層） | 本体にない | X1系のLiDAR・AIカメラの機能。P1Sは`xcam`を報告しない。 |

## 状態

| 機能 | 状態 | 入口・根拠 |
| --- | --- | --- |
| 温度・層・速度・ファン・照明 | 実装 | 状態APIの`live`。報告の差分を直前の全状態へ合成（Studioと同じ）。 |
| 本体エラー・HMS | 実装 | `print.error`と`live.hms`。公式解説へのリンク（HMS.cpp get_hms_wiki_url）、エラーの消去は操作`clear_error`。 |
| 報告の鮮度 | 実装 | `connection`の`stale`、`updated_at`。 |
| カメラ | 実装 | `GET /api/printers/{id}/camera`（ポート6000の静止画、ha-bambulab）。画面は更新ボタンで取得。 |
| 録画・タイムラプス設定 | 実装 | 操作`recording`・`timelapse`。`ipcam_record_set`・`ipcam_timelapse`。 |
| ファームウェア版 | 実装 | 接続ごとに`get_version`を送り、状態APIの`firmware`へ名前と版を出す。 |
| ファームウェア更新 | 本体で操作 | Studioはクラウドから得たURLを`upgrade.start`で送る（DevUpgradeCtrl.cpp）。クラウドの複製は対象外。本体の更新メニューかSDカードで行う。 |

## 機器・ファイル

| 機能 | 状態 | 入口・根拠 |
| --- | --- | --- |
| 印刷データの生成・転送 | 実装 | 公式OrcaSlicer CLIとimplicit FTPS（既存）。 |
| SDカードのファイル一覧・取得・削除 | 実装 | `GET/DELETE /api/printers/{id}/files`、`GET .../files/content`。印刷データの転送中は拒否。 |
| 複数台の非混線 | 実装 | 機器ごとに接続・状態・操作を分離。A1 mini 3台（AMS liteあり2台・なし1台）の隔離試験。 |
| 認証・LAN設定 | 本体で操作 | LANモード・アクセスコード・Wi-Fiは本体の設定画面で行う。サーバーは登録済みのアクセスコードと証明書だけを使う。 |

非対応の操作を受け付けたように見せません。本体が拒否した操作は`rejected`、応答がなければ`none`を返し、状態は後続の報告で確認します。
