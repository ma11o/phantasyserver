if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 704210)
        unlock_quest(sender, 704220)
        move_lobby(sender)
    end
end
