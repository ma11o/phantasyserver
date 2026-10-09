if call_type == "on_cutscene_end" then
    if zone == "cutscene" then
        clear_quest(sender, 700955)
        unlock_quest(sender, 700980)
        story_reward(sender, "", false)
        move_lobby(sender)
    end
end
