if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 703710)
        unlock_quest(sender, 703720)
        move_lobby(sender)
    end
end
