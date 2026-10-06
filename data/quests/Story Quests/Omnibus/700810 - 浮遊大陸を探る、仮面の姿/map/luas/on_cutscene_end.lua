if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 700810)
        unlock_quest(sender, 700820)
        move_lobby(sender)
    end
end
