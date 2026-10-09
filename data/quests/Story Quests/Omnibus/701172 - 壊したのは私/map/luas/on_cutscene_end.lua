if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 701172)
        unlock_quest(sender, 701173)
        story_reward(sender, "", false)
        move_lobby(sender)
    end
end
