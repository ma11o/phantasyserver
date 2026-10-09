if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 701810)
        unlock_quest(sender, 701830)
        move_lobby(sender)
    end
end
