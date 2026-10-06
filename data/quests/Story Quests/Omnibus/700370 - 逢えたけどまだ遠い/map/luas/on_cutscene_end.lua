if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        unlock_quest(sender, 700380)
        move_lobby(sender)
    end
end
