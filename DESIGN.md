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
ホームは保存済みプレートを検索して開く入口とする。新規作成はscad-liveのSTLを選び、
工程・材料・プレート種類と名前を指定して「配置して保存」で完了する。
名前は実行前に入力し、成功したら詳細へ移動する。取り込み後のスライスに失敗しても
取り込んだプレートを残し、同じIDで再試行する。選択や設定を修正した場合も同じIDへ保存する。成功表示は両工程の完了後だけに出す。
詳細は配置図、モデル名と設定、3MFのダウンロードを示す。印刷開始は未実装の間は表示しない。
保存内容を自動再取り込みせず、詳細から明示的に元モデルを再取得・再配置する。
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
  左にホームへ戻るアプリ名、右に36pxのメニューボタンを置く。リンクはアプリ名だけとする。
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

### プレート一覧と作成

ホームのURLは`/`。見出し「プレート」と主操作「新規作成」、名前・モデル名の検索欄、
保存済み一覧を順に置く。検索は入力中にサーバーのfuzzy順位で絞り込み、送信は不要。
各行は名前とモデル数・スライス状態だけを示す。先頭を210px以内、375×812で完全可視6件以上とする。
カードは10pxの内側余白、8pxの間隔。名前は折り返し、長いパスも横にはみ出させない。
一覧はページの縦スクロールを使う。Tab/Enterで開け、検索欄から下矢印でも候補へ移れる。

新規作成のURLは`/plates/new`。最初に検索付きのモデル一覧とcheckboxで複数選択する。
選択は検索文字を変えても保ち、選択数は次へ進む操作に一度だけ示す。
次に名前・工程・材料・プレート種類を確認し、選択へ戻る操作でも入力を保つ。
選択はSpace、候補移動は上下矢印/Tab。主操作は各段階に1つだけ置く。
設定はnative select、入力はラベル付きのnative inputを使う。

詳細のURLは`/plates/<id>`。再読込みで同じプレートを開き直す。
配置は保存3MFの変換を合成した実座標から、256mm四方の上面図に外形範囲を描く。
図は編集できず、形状の精密表示を示唆しない。面はaccent-subtle、境界はaccent、番号は文字で区別する。
図は最大320px四方で、狭幅では本文幅まで縮める。モデル名と寸法は通常の本文でも示す。
詳細では生成済み3MFの取得を主にし、元モデルの再取得は補助操作として分ける。
再取得すると現在の設定で配置・スライスを作り直すことを操作付近で伝える。

### 状態と復帰

一覧・モデル取得の状態はloading/empty/error/successとする。空の保存先と検索不一致を区別する。
読込中は14pxの文字とスピナー、失敗はrole=alertと同じ場所の再試行で示す。
scad-liveやOrcaSlicerが未設定でも、保存済みプレートの閲覧を遮らない。
配置処理中は「モデルを取り込んでいます」「配置・スライス中」をrole=statusで示し、
同じ処理を重ねて実行できないようフォームを無効にする。架空の進捗率を表示しない。
失敗時は入力や保存済みプレートを保持し、設定不足・サーバー使用中・時間超過に応じた復帰を示す。
検索結果は古い要求で新しい入力を上書きせず、移動後に結果を描画しない。

### 入力と操作部品

- 通常ボタンはsurface-raised、1px枠、6px角丸、上下8px・左右14pxの余白とする。
  ホバーはhover-1、無効時は不透明度50%でポインターを付けない。
- 主ボタンはaccentで塗り、静かなアイコンボタンは透明背景とする。
- 入力欄はsurface、1px枠、6px角丸、本文サイズとする。
  ラベルは上にmutedの注記で置き、フォーカスはaccent色の枠と共通リングを使う。
- モーダルは中央配置、12px角丸、16px余白とscrimを使う。
  閉じるボタン・Esc・scrimで閉じ、内容は内部でスクロールできる最大80dvhとする。
- 動きは150ms以下の高さ・透明度の変化とスピナーに限る。
  `prefers-reduced-motion: reduce` では両方を止める。

## Verification

実装はSassの字下げ構文を使い、normalize.cssを先に読み込む。
変数の接頭辞は色が `--c-*`、余白が `--sp-1..5`、文字が `--fs-xs..xl`、角丸が `--radius-*` とする。
`designmd lint` は形式を検査する。UIへの適用は実ブラウザで次を確認する。

- 同じデータ・画面サイズで変更前後を比べ、主要情報の面積と見える件数、色の強さを確認する。
  新機能が動いても、重複情報や強い装飾で主役が隠れたら修正する。
- 明暗、320px以上の狭幅、長い名前、空、読込、失敗、検索、キーボードを変更範囲に応じて確認する。
- Sumiの背景色をrgb(25,25,25)にする。Kinariではrgb(250,246,239)となる。
  明示指定がOSより優先され、自動へ戻すと属性と保存キーが消える。
- 375pxのheaderの操作対象はアプリ名とメニューボタンの2つだけとする。
  メニュー開閉時も横にはみ出さず、パネルの位置は指定する端と±1px以内で一致する。
- カード・アイコン・フォーカスの寸法、各状態、メニューとモーダルの閉じ方を部品規則と照合する。

通常フローと余白は `App.svelte` と `global.sass` の `.content` が所有する。
主操作・一覧・図は320px幅でも横にはみ出させない。
`npm --prefix client run test:e2e` で320px・375px・900pxの明暗を確認する。
100件と長い名前、取得失敗の再試行、選択・設定・保存・再検索・再読み込み・明示再取得を実測する。
テーマの保存とOS追従、キーボード、200%の文字拡大、メニューの閉じる操作を確認する。
