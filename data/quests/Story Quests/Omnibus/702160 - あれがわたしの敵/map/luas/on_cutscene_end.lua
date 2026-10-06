if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 702160)
        unlock_quest(sender, 702170)
        move_lobby(sender)
    end
end
