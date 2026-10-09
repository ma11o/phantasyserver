if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 701510)
        unlock_quest(sender, 701520)
        story_reward(sender, "", false)
        move_lobby(sender)
    end
end
