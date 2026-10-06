if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 704160)
        unlock_quest(sender, 704180)
        move_lobby(sender)
    end
end
