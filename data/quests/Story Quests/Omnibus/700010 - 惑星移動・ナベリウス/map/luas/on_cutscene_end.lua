if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 700010)
        unlock_quest(sender, 700020)
        move_lobby(sender)
    end
end
