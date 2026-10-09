if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 703980)
        unlock_quest(sender, 703990)
        move_lobby(sender)
    end
end
