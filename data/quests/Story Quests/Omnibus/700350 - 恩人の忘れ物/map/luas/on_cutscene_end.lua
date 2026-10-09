if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 700350)
        unlock_quest(sender, 700360)
        move_lobby(sender)
    end
end
