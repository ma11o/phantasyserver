if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 702970)
        unlock_quest(sender, 702980)
        move_lobby(sender)
    end
end
