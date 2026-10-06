if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 703330)
        unlock_quest(sender, 703340)
        move_lobby(sender)
    end
end
