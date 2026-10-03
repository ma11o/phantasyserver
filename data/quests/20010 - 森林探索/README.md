# 20010 (生成物。`tools/gen_quest.py 20010` で作る。手で直さない)

- 入力: free-field-catalog.md の行 (stage family [310, 311])、雛形 `200030 - Test Quest`、`tools/quest_overrides/20010.json`。
- `quest_obj.id` = 1201 (難易度側も同じ)。`quest_type` = Expedition。他の `unk*` と `difficulties` の中身は雛形のまま。
- ゾーン: campship (150) + ['campship_down=310', 'area2=311']。`args` は 310 のみ雛形の 1744、他は 0xFFFFFFFF (T23)。
- 対応表と変更理由: docs/findings/quest.md の「T24」。
