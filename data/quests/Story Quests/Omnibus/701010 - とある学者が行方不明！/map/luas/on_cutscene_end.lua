if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 701010)
        unlock_quest(sender, 701020)
        move_lobby(sender)
    end
end
