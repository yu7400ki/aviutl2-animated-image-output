# AviUtl2 アニメーション画像出力プラグイン

AviUtl ExEdit2 で動画をアニメーション画像として出力できるプラグインセットです。

## 対応フォーマット

- **PNG** (APNG)
- **GIF**
- **WebP**
- **AVIF**

## 動作環境

- AviUtl ExEdit2 beta3 以降

## インストール

1. [Release](https://github.com/yu7400ki/aviutl2-animated-image-output/releases) から最新版のプラグインファイルをダウンロード
2. ダウンロードした auo2 ファイルを `%ProgramData%\aviutl2\Plugin` フォルダにコピー
   - 例: `C:\ProgramData\aviutl2\Plugin\apng_output.auo2`
3. AviUtl2 を再起動

## 使い方

1. AviUtl2 でプロジェクトを開く
2. メニューから「ファイル」→「ファイル出力」を選択
3. 出力したいフォーマットを選択（例: APNG 出力プラグイン）
4. 設定ダイアログで出力オプションを調整
5. 出力ファイル名を指定して保存

## 各フォーマットの特徴

### PNG (APNG)

- **特徴**: 高品質、可逆圧縮、大きなファイルサイズ
- **用途**: 高品質なアニメーション画像

### GIF

- **特徴**: 広く対応、256 色制限
- **用途**: 互換性重視、軽量なアニメーション

### WebP

- **特徴**: 高圧縮率、可逆・非可逆両対応
- **用途**: ウェブ用、ファイルサイズを抑えたい場合

### AVIF

- **特徴**: 最高の圧縮率、最新フォーマット
- **用途**: 最小ファイルサイズ、最新環境

## 設定項目

各プラグインには以下の設定項目があります：

### PNG（APNG）出力設定

- **ループ回数**: アニメーションの繰り返し回数（0 = 無限ループ）
- **カラーフォーマット**: RGB 24bit / RGBA 32bit
- **圧縮**: 標準 / 高速 / 最高
- **フィルター**: PNG のフィルター設定（なし、Sub、Up、Average、Paeth）

### GIF 出力設定

- **ループ回数**: アニメーションの繰り返し回数（0 = 無限ループ）
- **カラーフォーマット**: RGB 24bit / RGBA 32bit
- **パレット生成速度**: NeuQuant アルゴリズムの処理速度（1-30、高い値=速い処理・低品質）

### WebP 出力設定

- **ループ回数**: アニメーションの繰り返し回数（0 = 無限ループ）
- **カラーフォーマット**: RGB 24bit / RGBA 32bit
- **ロスレス圧縮**: 可逆圧縮の ON/OFF
- **品質**: 画質設定（0-100、ロスレス OFF 時のみ）
- **圧縮方法**: 圧縮アルゴリズム（0-6、ロスレス OFF 時のみ）

### AVIF 出力設定

- **品質**: 画質設定（0-100）
- **速度**: エンコード速度（0-10、値が大きいほど高速）
- **カラーフォーマット**: RGB 24bit / RGBA 32bit

## 注意事項

- 大きなファイルサイズや長時間の動画では処理時間が長くなる場合があります
