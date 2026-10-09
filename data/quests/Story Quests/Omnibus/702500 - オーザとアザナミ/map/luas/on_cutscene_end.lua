if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 702500)
        unlock_quest(sender, 702510)
        move_lobby(sender)
    end
end
