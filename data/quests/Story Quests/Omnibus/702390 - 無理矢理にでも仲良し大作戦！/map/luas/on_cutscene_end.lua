if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 702390)
        unlock_quest(sender, 702400)
        move_lobby(sender)
    end
end
