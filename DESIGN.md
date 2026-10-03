---
version: alpha
name: OrcaServer / Sumi
description: OrcaServerのプレート選択・配置・保存と明暗テーマのデザイン契約。
colors:
  primary: "#245d84"
  accent: "#245d84"
  accent-subtle: "rgba(36, 93, 132, 0.10)"
  surface: "#faf6ef"
  surface-raised: "#fffdf8"
  on-surface: "#3a2f28"
  muted: "#6f6257"
  border: "#e3d9c9"
  scrim: "rgba(58, 47, 40, 0.4)"
  link: "#14506e"
  danger: "#9c2b1d"
  danger-subtle: "#f9e9e4"
  wash-base: "#e7eff4"
  wash-raised: "#f0f4f7"
  hover-1: "rgba(36, 93, 132, 0.10)"
  hover-2: "rgba(36, 93, 132, 0.16)"
typography:
  title:
    fontFamily: system-ui
    fontSize: 17px
    fontWeight: 600
    lineHeight: 1.3
  body:
    fontFamily: system-ui
    fontSize: 16px
    fontWeight: 400
    lineHeight: 1.6
  body-sm:
    fontFamily: system-ui
    fontSize: 14px
    fontWeight: 400
    lineHeight: 1.5
  label:
    fontFamily: system-ui
    fontSize: 15px
    fontWeight: 500
    lineHeight: 1.2
  caption:
    fontFamily: system-ui
    fontSize: 12px
    fontWeight: 400
    lineHeight: 1.4
rounded:
  sm: 6px
  md: 8px
  lg: 12px
  full: 9999px
spacing:
  sp-1: 4px
  sp-2: 8px
  sp-3: 12px
  sp-4: 16px
  sp-5: 24px
components:
  app-header:
    backgroundColor: "{colors.wash-base}"
    textColor: "{colors.on-surface}"
    height: 48px
  sub-header:
    backgroundColor: "{colors.wash-raised}"
    textColor: "{colors.on-surface}"
    height: 40px
  hairline:
    backgroundColor: "{colors.border}"
    height: 1px
  card:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.on-surface}"
    rounded: "{rounded.md}"
    padding: 10px
  button:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.on-surface}"
    typography: "{typography.label}"
    rounded: "{rounded.sm}"
    padding: 8px
  button-hover:
    backgroundColor: "{colors.hover-1}"
  button-pressed:
    backgroundColor: "{colors.hover-2}"
  button-primary:
    backgroundColor: "{colors.primary}"
    textColor: "{colors.surface-raised}"
    typography: "{typography.label}"
    rounded: "{rounded.sm}"
    padding: 8px
  button-quiet:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.muted}"
    rounded: "{rounded.sm}"
  icon-button:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.on-surface}"
    rounded: "{rounded.sm}"
    size: 36px
  input:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.on-surface}"
    typography: "{typography.body}"
    rounded: "{rounded.sm}"
    padding: 8px
  modal:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.on-surface}"
    rounded: "{rounded.lg}"
    padding: 16px
  modal-scrim:
    backgroundColor: "{colors.scrim}"
  radio-selected:
    backgroundColor: "{colors.accent-subtle}"
    rounded: "{rounded.sm}"
  link:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.link}"
  error-banner:
    backgroundColor: "{colors.danger-subtle}"
    textColor: "{colors.danger}"
    typography: "{typography.body-sm}"
    rounded: "{rounded.sm}"
    padding: 8px
  spinner:
    textColor: "{colors.accent}"
    size: 18px
  badge:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.muted}"
    typography: "{typography.caption}"
    rounded: "{rounded.full}"
    padding: 4px
---

# OrcaServer

## Overview

OrcaServerのブラウザ画面を定める自己完結した契約。
Rust + Svelte Templateの原本（2026-09-20参照）を基に、製品が所有する。
ホームはプリンターと印刷キューの入口とし、保存済みプレートは`/plates`で検索して開く。プレートは名前・3MF/STLの参照・個数と、要求機種/ノズル・役割別の材料・工程/品質・ビルドプレートを保存する。
新規作成はモデル選択→名前・個数・印刷条件の確認→保存→詳細。詳細内の操作で直接キューへ追加する。
材料・機種/ノズル・品質・ベッド条件はプレートで指定し、詳細から直接キューへ追加する。
プレート保存後、キューがなくても最新モデルと現在のプロファイルからバックグラウンドで配置・スライスする。保存は計算を待たず詳細へ戻る。
詳細の名前直下とキュー各行は同じ推定所要時間を表示する。実機の接続・待機やAMSへの装填を試算の条件にしない。
試算の待ち・計算中・時間・失敗は名前の下の注記で区別し、失敗の詳細を開くと原因・修正先・再試算を示す。
主操作の印刷開始・継続は従来どおり1ボタン。試算完了を印刷可能・印刷開始済みと扱わず、取り外し操作も行わない。
条件変更後は以前の時間を隠し再計算する。総時間や取り外し待ちを含む完了予定時刻は表示しない。
開始時にも最新のSCADモデルと現在のプロファイルを照合し、一致する試算の生成物だけを再利用する。それ以外は再配置・スライスする。
同じ入力の結果はプレートに保持し、名前だけの編集、キュー追加・削除、再表示・再起動で作り直さない。SCAD公開元や材料設定の更新も再評価する。
取得失敗時は古いデータへ戻さず要確認にする。準備開始後のモデル・設定はその実行用に固定する。
右上のメニューからテーマ設定を開き、変更結果をその場で確認して画面へ戻る。
情報を重複させる説明帯や、将来の機能を予告するパネルは置かない。
アクセントは青の明暗の組とし、テーマ保存キーは `orca-server:theme` とする。

## Colors

暗色はSumi、通常画面の明色はKinariとする。Sumiから設計し、両方で検証する。
色は `client/src/global.sass` のCSS変数を使う。frontmatterはKinariの値を持つ。

| 役割 | Kinari | Sumi |
|---|---|---|
| surface | #faf6ef | #191919 |
| surface-raised | #fffdf8 | #232323 |
| on-surface | #3a2f28 | #e6e6e6 |
| muted | #6f6257 | #9a9a9a |
| border | #e3d9c9 | #333333 |
| accent / primary | #245d84 | #80c8ed |
| accent-subtle | rgba(36,93,132,.10) | rgba(128,200,237,.15) |
| link | #14506e | #7fdbff |
| danger | #9c2b1d | #ff6b6b |
| danger-subtle | #f9e9e4 | #3a1a1a |
| scrim | rgba(58,47,40,.4) | rgba(0,0,0,.6) |
| wash-base | #e7eff4 | #232323 |
| wash-raised | #f0f4f7 | #191919 |
| hover-1 | rgba(36,93,132,.10) | #333333 |
| hover-2 | rgba(36,93,132,.16) | #3d3d3d |

`primary` はlint用に `accent` と同値を持つ。製品の色名はaccentを使う。
背景・補助情報・通常操作は無彩色を基本とする。Kinariでは表の淡い色を許す。
帯にはwash、ホバーにはhoverの変数を使い、accent-subtleを直接流用しない。
アクセントは主操作・フォーカス・小さな選択表示・処理中のスピナーに使う。
塗りつぶす主操作は画面に最大1つとし、領域を分けて増やさない。
大きな選択行は背景の濃淡で示し、強いアクセントの面と縁取りを重ねない。
小さなラジオの選択表示にはaccent-subtleを使える。

副アクセントは、主アクセントと異なる持続的な役割がある場合だけ製品側で定める。
分類などのデータ色は操作の色と分け、製品側で用途を定める。
色の意味は文字・形・アクセシビリティ属性でも示す。
通常文字は両テーマでWCAG AAの4.5:1以上を保つ。主ボタンの文字色はsurface-raisedとする。

### テーマの状態

`:root` をSumiの値と `color-scheme: dark` にする。
`data-theme="light"` でKinari、`data-theme="dark"` でSumiを明示指定する。
自動では属性と保存キーを削除し、OSの `prefers-color-scheme` に従う。
Kinariの明示指定とOS委任は同じSass mixinから出力し、`color-scheme: light` を設定する。
設定はlocalStorageへ保存し、初回描画前に適用する。状態をコンポーネント内だけに保持しない。

## Typography

書体はsystem-uiとし、Webフォントを追加しない。サイズ・太さ・行高はfrontmatterの5役割を使う。
headerのアプリ名は1行で省略し、プレート名は折り返す。本文は16px以上とする。補助文は14px、ラベルは15px、注記は12pxを使う。
注記はデータ色を持つ場合を除いてmutedとする。階層は色と太さで示し、独自サイズを増やさない。

## Layout

### 内容と補助情報

一覧や本文を最初の画面で見せる。現在地・選択対象・件数が既存の表示で分かるなら、説明帯を追加しない。
検索・主要操作は見出しの近くへまとめ、ヘルプ・確認・詳細説明は必要な操作から開く。
異常時の通知は必要だが、通常時に空の通知領域を予約しない。

縦方向はページの通常スクロールを使う。表の列見出しも行と一緒に流す。
固定見出しのための内側縦スクロール、高さ制限、残り高さの計算を標準の一覧へ持ち込まない。
狭幅の表は必要な横スクロールだけを表内へ収める。
カード向けの幅を密な表や全画面の画像へ適用しない。製品ごとの用途をroot DESIGN.mdで定める。

### 画面構成

- app headerは全幅・高さ48px・stickyとし、wash-baseと1pxの下境界を使う。
  左に「OrcaServer」「プレート」の2タブ、右に36pxのメニューボタンを置く。
  タブは`/`と`/plates`へ移り、現在地を下線とaria-currentで示す。
- 本文は通常の文書フローで続く。mainに独立した縦スクロール領域を作らない。

これらの既存headerは、表の列見出しとは別の部品である。
ブレークポイントは768px。カード・本文の列は中央配置で最大720pxとする。
左右の余白は12px、上下は狭幅16px・広幅24pxとする。幅320px以上でページを横にはみ出させない。
余白は4/8/12/16/24pxを使う。カード内10px、通常ボタンの横14pxは部品固有の値とする。

## Elevation & Depth

階層は面の濃淡と1pxの境界で示す。
影はメニューとモーダルの `0 8px 32px rgba(0,0,0,.25)` だけに使う。
フォーカスは共通の `:focus-visible` に2pxのaccent色の輪郭と2pxの間隔を設ける。
ブラウザ既定の輪郭を抑止する場合も、この可視リングを残す。

## Shapes

角丸は小部品6px、カード8px、モーダルとメニュー12pxとする。
9999pxは件数と状態のバッジだけに使う。同じ操作部品で角丸を混ぜず、円形ボタンを作らない。

## Components

### 引き算の原則

2026-10-01にユーザーは、文章を盛った画面と文字のボタンを問題とし、アイコンと開閉で引き算するよう求めた。
初期表示は対象・状態・主操作に絞り、説明文の段落を置かない。補足・詳細パラメータ・履歴・長い原因は既定で閉じた開閉へ入れる。
更新・編集・削除・詳細・移動・取得・外部リンクなどの定型操作はアイコンだけで示し、accessible nameとtitleを付ける。文字記号をアイコンの代わりにしない。
主操作は短い動詞（印刷・次を印刷・再印刷・除外・保存・追加）とする。押下が空のプレートの確認を兼ねる契約を保ち、確認ダイアログを足さない。
利用者が求めた情報（エラー番号と短い意味、外部給材の選択、AMSの取得結果）は初期表示に残す。文字を小さくする、画面外へ押し出す、別の場所へ移すだけで減らさない。
ボタン文言・取得間隔・要求IDなどの実装の詳細を設計規則にしない。新しい画面を作ったという事実だけで規則にしない。

### アイコン

`client/src/lib/Icon.svelte` を辞書の正とする。絵文字や文字記号をアイコンの代わりに使わない。
SVGは24×24、currentColorの2px線、丸い端と角、通常は塗りなしとする。
サイズは1.2emで文字の基準線にそろえる。filled版は同じ形状を塗り、状態は属性でも示す。
列挙には `ICON_NAMES` を使い、別の手書き一覧を作らない。未使用だけを理由に辞書項目を削らない。
汎用的な追加は原本の辞書へ採用し、派生製品が明示的に取り込む。実行時依存やsubmoduleは不要。

### メニューとテーマ設定

メニューは右上のボタンに接するドロップダウンとする。
上端をheader下端、右端をボタン右端へそろえる。最小幅180px、1pxの枠と12pxの角丸を使う。
背景はsurface-raised、項目は全幅の行とし、余白は上下8px・左右12pxを使う。
メニューにscrimは付けない。背面の透明な閉じるボタンで外側クリックを受ける。
Escでも閉じ、フォーカスをメニューボタンへ戻す。開閉は `aria-expanded` に反映する。
先頭は「テーマ設定」、以降は製品の画面リンクとする。ホーム項目は重複するため置かない。
「ライセンスとソース」から`/about`へ移動する。版・著作権・無保証・利用許諾、本文と第三者通知、
ビルドに対応するソースの取得先を通常の本文とリンクで示す。塗りつぶしの主操作は置かない。
取得失敗は再試行でき、公開先未設定の開発ビルドを公式版のソースへ案内しない。

テーマ設定は中央のモーダルで開く。
自動・ライト・ダークの3つのラジオにmonitor/sun/moonのアイコンを使う。
選択は即時反映し、確認できるようモーダルを閉じない。
閉じるボタン・Esc・scrimで閉じ、メニューボタンへフォーカスを戻す。

### プレート

一覧（`/plates`）は見出し・新規作成・検索・保存済みの行だけを置く。行は名前とモデル数・個数に絞る。
新規作成は検索付きのモデル選択→名前・個数・印刷条件→保存→詳細の順とし、ファイル取込はSCAD接続に依存させない。
構成の編集は同じ画面で行い、モデルの追加・差し替え・外すを行内に置く。モデルは64個まで。
印刷条件は機種/ノズル・役割ごとの材料・工程・ビルドプレートを常時示し、インフィル・ブリム・サポートは閉じた「詳細設定」、開始オプションは閉じた「印刷開始」に入れる。
材料は検索モーダルで選び、初期候補は所持機の装填材料とする。全台帳の検索は一時的な切替で、プレートには保存しない。
詳細（`/plates/<id>`）は名前・試算時間・キューへの追加・保存した条件・モデルを左、選択中の3D形状を右（狭幅は下）に置く。
主操作は「印刷キューへ」、構成の編集・形状の操作・元ファイルの取得はアイコンとする。
試算の失敗は原因・修正先・再試算を開いた先に示し、条件を直して保存すれば再計算する。
プレート行の右クリック・長押し・ContextMenuキーで、キュー追加・編集・複製・削除の文脈メニューを開く。削除は一度だけプレート名を含めて確認する。
削除は一覧から外す操作で、既存のジョブと元データは残す。

### 印刷キュー

ホームはプリンターごとに名前・本体操作と設定のアイコン・温度と層の1行・現在のジョブ・主操作・待機一覧を並べる。
ジョブは折り畳んだ2行（名前と状態または試算時間）とし、開くと材料・給材・工程・ベッド・理由と編集/削除のアイコンを示す。
主操作は状態ごとに1つ：待機の先頭を「印刷」、取り外し待ちから「次を印刷」、待機がなければ「取り外した」。要確認では「再印刷」と補助の「除外」。
押下は空のプレートの確認を兼ね、独立したcheckboxや確認ダイアログは置かない。操作できない理由は操作の近くに短く示す。
本体が報告した失敗は、コードと短い意味の1行と公式解説のアイコンだけを現在ジョブの下に出す。本体の現在の報告と保存済みの過去の報告を「現在」「前回」で区別する。
意味は公式で確認したものだけを示し（0300-8010＝ホットエンド冷却ファンの回転異常）、未確認のコードは未確認と書く。状態や理由の詳細はジョブを開いた先に置く。
印刷中は一時停止または再開と停止のアイコンを置き、停止は2回目の押下で確定する。
待機の並べ替えはハンドルのドラッグとキーボードで行う。行の文脈メニューにプレート編集・キュー複製・給材の切替・キュー削除を置く。
給材はキュー追加でAMS・外部スプールを選び、AMSを報告しない実機では外部スプールを初期値とする。AMSが見えなくなっただけで既存のジョブの給材を変えない。
応答が不明な操作は他の操作を止め、同じ要求の結果を確認する。完了や閲覧だけで次の印刷を開始しない。

### 本体の操作

本体操作ページの初期表示は名前・状態の1行・一時停止/再開・停止・照明・カメラのアイコンに絞る。
温度・ファン・速度、移動、給材、オプション、キャリブレーション、ファイル、本体の情報は既定で閉じた開閉に入れる。
本体が対応しない設定は出さず、未報告の設定は変更できない状態で示す。
送信の結果は受付・拒否と理由・応答なしを区別して1行で示し、状態は本体の次の報告で確かめる。
停止とファイル削除は2回目の押下で確定する。本体のエラーとHMSはコードと公式解説のアイコンで示す。

### プリント履歴

キューの末尾から開く独立ページとし、印刷時のプレート名と正常完了の絶対日時だけを並べる。
過去分は追加で読み、読み込んだ行と位置を保つ。行の文脈メニューから現在のプレートを同じ追加契約でキューへ戻す。削除済みのプレートは追加できない。

### 状態と復帰

読込中・空・失敗を区別し、失敗はその場に再試行を置く。空の保存先と検索の不一致を分ける。
外部サービスやスライサーが未設定でも保存済みの閲覧を遮らない。架空の進捗率を表示しない。
失敗で入力や保存済みの内容を失わず、古い読取りで新しい入力や操作結果を上書きしない。

### 入力と操作部品

- 通常ボタンはsurface-raised、1px枠、6px角丸、上下8px・左右14pxの余白とする。
  ホバーはhover-1、無効時は不透明度50%でポインターを付けない。
- 主ボタンはaccentで塗り、静かなアイコンボタンは透明背景とする。
- 入力欄はsurface、1px枠、6px角丸、本文サイズとする。
  ラベルは上にmutedの注記で置き、フォーカスはaccent色の枠と共通リングを使う。
- モーダルは中央配置、12px角丸、16px余白とscrimを使う。
  閉じるボタン・Esc・scrimで閉じ、内容は内部でスクロールできる最大80dvhとする。
  材料検索では検索入力を初期フォーカスとし、Tabを内部に保つ。選択・キャンセル・全ての閉じ方で起点の材料ボタンへ戻す。
- 動きは150ms以下の高さ・透明度の変化とスピナーに限る。
  `prefers-reduced-motion: reduce` では両方を止める。

### プリンター

一覧は機器ごとに名前・機種とノズル・接続状態と、AMS・キューへの短いリンク、本体操作と設定のアイコンを置く。「追加」を唯一の塗りボタンとする。
新規プレートの初期値に使う機器を一覧で選び、詳細初期値は閉じた開閉に入れる。
編集は名前・機種とノズル・新規プレートの初期値を先に置き、LAN接続（アドレス・シリアル・アクセスコード・証明書・ポート）は既存機器では閉じた開閉に入れる。空欄のアクセスコードと証明書は現在の値を維持する。
印刷中や待機中のジョブがある機器は削除できず、拒否の理由をその場に示す。

### 材料とAMS

材料は製品（メーカー・材質・機種別設定）と色の2段で管理する。一覧は製品名・メーカー・材質と色見本・色名を示し、「製品を追加」を唯一の主ボタンとする。
製品詳細は色と機種別設定を並べ、「色を追加」を主操作、共通情報の編集をアイコンとする。既存の色をまとめる操作と削除は閉じた開閉に入れる。
色の正確な値・温度の差分の説明は編集画面の開閉に置く。0℃のベッド温度は保存できるが、そのビルドプレートでは追加・試算を止めて設定先を示す。
AMS画面は番号・材料と色・読取状態を一行に並べ、材料の選択と詳細の開閉を同じ行に置く。温度・タグ・残量・観測時刻・使用順・補充先は詳細に入れる。
状態の取得はアイコンで本体へ要求し、新しい報告が保存されてから反映する。取得中・完了時刻・読取中・失敗の理由を見出しの下に示し、失敗は同じアイコンで再試行する。
初回の割当は全台帳から検索する。観測が変わったら選び直しを求める。自動補充は閉じた開閉に入れ、本体の報告による対応・非対応・未報告と有効状態を示す。

## Verification

実装はSassの字下げ構文を使い、normalize.cssを先に読み込む。
変数の接頭辞は色が `--c-*`、余白が `--sp-1..5`、文字が `--fs-xs..xl`、角丸が `--radius-*` とする。
`designmd lint` は形式を検査する。UIへの適用は実ブラウザで次を確認する。

- 同じデータ・画面サイズで変更前後を比べ、初期表示の文字数、説明の段落、文字のボタン、主操作までの操作回数が増えていないことを確かめる。
  新機能が動いても、重複情報や強い装飾で主役が隠れたら修正する。
- 明暗、320px以上の狭幅、長い名前、空・読込・失敗、キーボード、文字200%を変更範囲に応じて確認する。
- アイコンの名前、開閉とフォーカス、メニューとモーダルの閉じ方を部品規則と照合する。
- Sumiの背景色はrgb(25,25,25)、Kinariはrgb(250,246,239)。明示指定はOSより優先し、自動へ戻すと保存を消す。

通常フローと余白は `App.svelte` と `global.sass` の `.content` が所有する。主操作・一覧・図は320px幅でも横にはみ出させない。
