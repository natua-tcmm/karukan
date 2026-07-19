# 変換モデルのライセンス

Karukanは初回インストール時または初回変換時に、次のGGUFモデルと
`tokenizer.json`をHugging Faceから取得します。モデルファイルはアプリ本体には
同梱されません。

| モデル | 配布元 | ライセンス |
|---|---|---|
| jinen-v1-small | [togatogah/jinen-v1-small](https://huggingface.co/togatogah/jinen-v1-small) | [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) |
| jinen-v1-xsmall | [togatogah/jinen-v1-xsmall](https://huggingface.co/togatogah/jinen-v1-xsmall) | [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/) |

各モデルはtogatogahによって開発され、
`Miwa-Keita/zenz-v2.5-dataset`を前処理したデータで学習されています。モデルを
アプリやインストーラーへ同梱して再配布する場合は、配布時点のモデルカードと
CC BY-SA 4.0の表示・継承条件を改めて確認してください。
