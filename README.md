<div align="center">
  <img src="icon.png" width="128" alt="Karukan" />
  <h1>Karukan</h1>
  <p>macOS向けのニューラル日本語入力システム</p>

  [![macOS CI](https://github.com/natua-tcmm/karukan/actions/workflows/karukan-macos-ci.yml/badge.svg)](https://github.com/natua-tcmm/karukan/actions/workflows/karukan-macos-ci.yml)
  [![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
</div>

<div align="center">
  <img src="images/demo.gif" width="800" alt="Karukanの変換デモ" />
</div>

## 特徴

- 入力と同時に候補を表示するライブ変換
- 文脈を考慮するニューラルかな漢字変換
- 文節ごとの候補選択・境界調整
- 修正した文節を左右の文脈とともに学習

## インストール

macOS 13以降と、Rust・Swiftの開発環境が必要です。

```bash
git clone https://github.com/natua-tcmm/karukan.git
cd karukan/karukan-macos
make install
```

初回のみmacOSからログアウトして再ログインし、「システム設定」→「キーボード」→
「入力ソース」からKarukanを追加してください。

詳しい手順、更新方法、設定項目は
[macOS版README](karukan-macos/README.md)を参照してください。

## ライセンス

このリポジトリは[togatoga/karukan](https://github.com/togatoga/karukan)を基にした
forkです。ソースコードは原著作者の表示を保持し、
[MIT License](LICENSE-MIT)または[Apache License 2.0](LICENSE-APACHE)の条件で
利用・改変・再配布できます。

Mozc由来データや形態素解析辞書などの表示は
[THIRD_PARTY_LICENSES](THIRD_PARTY_LICENSES)、Rust依存ライブラリは
[THIRD_PARTY_CARGO_LICENSES.html](THIRD_PARTY_CARGO_LICENSES.html)、変換モデルは
[MODEL_LICENSES.md](MODEL_LICENSES.md)を参照してください。システム辞書を再配布する
場合は、別途[辞書ライセンス確認表](docs/dictionary-licenses.md)に従ってください。
公開前の確認事項は[再配布チェックリスト](docs/redistribution.md)にまとめています。
