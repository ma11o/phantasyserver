if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 701130)
        unlock_quest(sender, 701132)
        move_lobby(sender)
    end
end
