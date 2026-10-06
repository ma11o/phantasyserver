-- [pso2_vita_offline] provisional: copy of 700000 with the scene picked by the client's event chronicle title (st_010220).
if call_type == "on_map_loaded" then
    if zone == "landing" then
        move_player(sender, "cutscene")
    elseif zone == "cutscene" then
        local cutscene_data = {}
        cutscene_data.scene_name = "st_010220_om"
        cutscene_data.unk5 = 6
        cutscene_data.unk7 = 14
        local packet = {}
        packet.StartCutscene = cutscene_data
        send(sender,packet)
    end
end
