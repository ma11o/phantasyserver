# 20040 (生成物。`tools/gen_quest.py 20040` で作る。手で直さない)

- 入力: free-field-catalog.md の行 (stage family [321, 320])、雛形 `200030 - Test Quest`、`tools/quest_overrides/fields.csv` の行と `20040.json` (有れば)。
- `quest_obj.id` = 1204 (難易度側も同じ)。`quest_type` = Expedition。難易度 (受注 Lv・敵 Lv) は fields_difficulty.csv: [('N', 1, 24), ('H', 20, 35), ('VH', 40, 45), ('SH', 50, 64)]。他の `unk*` と `difficulties` の残りの欄は雛形のまま。
- ゾーン: campship (150) + ['campship_down=321', 'area2=320']。`args`・着地座標は docs/stage-spawn.csv (T25)。
- 対応表と変更理由: docs/findings/quest.md の「T24」。
