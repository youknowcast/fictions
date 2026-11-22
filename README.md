# fictions

受け取った動画・音声を、ローカルで再生する前に一度通すための CLI。
人から渡されたファイルを QuickTime で開ける形に正規化し、メタデータを削り、
サンドボックス内の ffmpeg で処理する。

## 必要なもの

- Rust（`mise install` で入る）
- ffmpeg 6.0 以上（`brew install ffmpeg`）

## インストール

```bash
mise install
cargo build --release
# できた target/release/fictions を PATH の通った場所へ
```

## 使い方

```bash
fictions ~/Downloads/foo.mp4        # 同じ場所に foo-<hash>.mp4 ができる
fictions --dry-run foo.mp4          # 何が出力されるかだけ見る
fictions --force foo.mp4            # 既存の出力を上書きする
fictions --no-group foo.mp4         # 連結せず、そのファイルだけ処理する
```

出力のパスだけが stdout に出る。進捗・警告・エラーは stderr。

### 連結

`foo.mp4`, `foo (1).mp4`, `foo (2).mp4` のように、同じ名前で連番が付いた
ファイルは 1 本の動画の分割パーツとみなし、番号順に連結する。
中身が完全に同じパーツは重複ダウンロードとみなして 1 本に畳む。

`Movie (1982).mp4` の年のような括弧数字は、`Movie` というファイルが
同じディレクトリに無ければ分割番号として扱わない。

## 出力

| 入力 | 映像 | コンテナ |
| --- | --- | --- |
| 1080p 以下・SDR | H.264 High / yuv420p / CRF 21 | mp4 |
| 1080p 超 または HDR | HEVC / `hvc1` / CRF 23 | mp4 |
| 音声のみ | なし（AAC-LC 48kHz） | m4a |

- 解像度は変えない。偶数への丸めと pixel aspect の 1:1 化だけ行う。
- 映像は先頭の1本、音声は先頭の1本だけ残す。字幕・チャプター・添付・
  メタデータは削除する。
- 出力名は `<元の名前の先頭50文字>-<入力内容のハッシュ8桁>.<拡張子>`。
  同じ入力なら同じ名前になるので、再実行はスキップになる。
- 入力ファイルは変更も削除もしない。

## サンドボックス

macOS では `sandbox-exec` で ffmpeg を包む。

- ネットワークは使えない
- 書き込みは一時ディレクトリだけ
- 読み取りはシステム領域・入力ファイル・ffmpeg/ffprobe の実体だけ
  （パス解決のためのメタデータ参照は全体に許可する）
- 起動時にサンドボックス内で `ffprobe -version` を実行し、失敗したら
  「サンドボックスが ffprobe を起動できない」と明示して止まる

Linux では `bwrap` を使う（読み取りはファイルシステム全体を読み取り専用）。

`sandbox-exec` も `bwrap` も無い環境では、黙って隔離を外さずに失敗する。
どうしても必要なら `--no-sandbox` を付けるが、信頼できない入力には使わないこと。

## 壊れた入力

途中で切れたダウンロードなどでデコードエラーが出た場合、出力は残すが
終了コード 1 で「疑わしい」と報告する。再生できるかもしれないが保証はしない。

## 既知の制限

- HDR は HEVC のまま通すだけで、トーンマッピングはしない。
- 副音声・字幕・チャプターは残せない。
- パーツ間で HDR/SDR や bit depth が混在するグループは変換できない
  （`--no-group` で個別に処理する）。
- 4K HEVC の再エンコードは遅い。
- 複数ファイルの一括処理は shell 側で行う。
- macOS の `sandbox-exec` プロファイルは起動時の `ffprobe` 自己診断だけで
  担保している。自己診断に失敗する環境では `--no-sandbox` を試す。

## 開発

```bash
cargo test          # ffmpeg が必要（フィクスチャを生成して統合テストする）
cargo clippy -- -D warnings
cargo fmt --check
```

macOS のサンドボックスは Linux の CI では検証できないため、Mac で一度
`sandbox-exec` 込みの実行を確認すること。
