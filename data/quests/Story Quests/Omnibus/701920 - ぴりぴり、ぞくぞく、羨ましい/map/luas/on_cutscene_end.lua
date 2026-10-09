if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 701920)
        unlock_quest(sender, 701930)
        move_lobby(sender)
    end
end
