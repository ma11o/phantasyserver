if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 703380)
        unlock_quest(sender, 703390)
        move_lobby(sender)
    end
end
