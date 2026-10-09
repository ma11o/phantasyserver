if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 701760)
        unlock_quest(sender, 701770)
        move_lobby(sender)
    end
end
