if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 702410)
        unlock_quest(sender, 702420)
        story_reward(sender, "", false)
        move_lobby(sender)
    end
end
