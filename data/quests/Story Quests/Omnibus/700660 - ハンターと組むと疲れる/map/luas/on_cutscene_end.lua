if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 700660)
        unlock_quest(sender, 700670)
        move_lobby(sender)
    end
end
