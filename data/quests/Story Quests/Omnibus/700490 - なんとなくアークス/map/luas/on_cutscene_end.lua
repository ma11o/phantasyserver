if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 700490)
        unlock_quest(sender, 700500)
        move_lobby(sender)
    end
end
