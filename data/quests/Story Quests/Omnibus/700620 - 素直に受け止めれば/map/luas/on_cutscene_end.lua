if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 700620)
        unlock_quest(sender, 700630)
        move_lobby(sender)
    end
end
