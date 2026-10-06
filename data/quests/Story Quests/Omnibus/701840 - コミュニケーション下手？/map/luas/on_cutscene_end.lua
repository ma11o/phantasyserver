if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 701840)
        unlock_quest(sender, 701870)
        move_lobby(sender)
    end
end
