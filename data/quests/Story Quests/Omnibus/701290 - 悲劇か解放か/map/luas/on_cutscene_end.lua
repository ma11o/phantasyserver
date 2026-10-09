if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 701290)
        unlock_quest(sender, 701300)
        story_reward(sender, "", false)
        move_lobby(sender)
    end
end
