if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 703990)
        unlock_quest(sender, 704000)
        move_lobby(sender)
    end
end
