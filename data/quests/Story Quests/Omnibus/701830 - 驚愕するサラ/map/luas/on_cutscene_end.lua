if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 701830)
        unlock_quest(sender, 701840)
        move_lobby(sender)
    end
end
