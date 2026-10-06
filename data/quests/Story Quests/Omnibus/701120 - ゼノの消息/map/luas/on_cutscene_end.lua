if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 701120)
        unlock_quest(sender, 701130)
        move_lobby(sender)
    end
end
