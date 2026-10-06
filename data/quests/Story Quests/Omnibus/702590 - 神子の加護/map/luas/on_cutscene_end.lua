if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 702590)
        unlock_quest(sender, 702600)
        move_lobby(sender)
    end
end
