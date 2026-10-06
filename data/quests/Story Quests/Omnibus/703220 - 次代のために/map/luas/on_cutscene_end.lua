if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 703220)
        unlock_quest(sender, 703230)
        move_lobby(sender)
    end
end
