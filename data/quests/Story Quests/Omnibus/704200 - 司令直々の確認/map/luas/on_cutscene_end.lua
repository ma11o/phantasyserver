if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 704200)
        unlock_quest(sender, 704210)
        move_lobby(sender)
    end
end
