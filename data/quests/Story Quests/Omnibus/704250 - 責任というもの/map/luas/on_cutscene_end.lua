if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 704250)
        unlock_quest(sender, 704260)
        move_lobby(sender)
    end
end
