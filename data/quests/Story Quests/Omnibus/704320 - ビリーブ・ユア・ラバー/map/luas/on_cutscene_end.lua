if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 704320)
        unlock_quest(sender, 704330)
        move_lobby(sender)
    end
end
