if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 703820)
        unlock_quest(sender, 703830)
        move_lobby(sender)
    end
end
