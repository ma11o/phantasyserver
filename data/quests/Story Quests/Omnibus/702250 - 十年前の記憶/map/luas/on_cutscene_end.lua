if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 702250)
        unlock_quest(sender, 702260)
        story_reward(sender, "", false)
        move_lobby(sender)
    end
end
