if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 703610)
        unlock_quest(sender, 703620)
        move_lobby(sender)
    end
end
