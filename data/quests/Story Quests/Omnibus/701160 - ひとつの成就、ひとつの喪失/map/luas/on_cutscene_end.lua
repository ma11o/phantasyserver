if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 701160)
        unlock_quest(sender, 701170)
        move_lobby(sender)
    end
end
